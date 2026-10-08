// SPDX-License-Identifier: GPL-2.0

//! estiba — dispositivo de bloque con RAM comprimida en Rust (zram-like).
//!
//! Disco `estiba` de 4 MiB, slot de 4 KiB, codec v1 (LZ + valores + Huffman
//! canónico) del crate `codec/`, vendoreado en `gen_codec/` (generado por
//! `gen-codec.sh`; equivalencia bit-exacta validada por `parity/`).
//!
//! Modelo de I/O: `queue_rq` síncrono recorre la cadena de bios del
//! `struct request` crudo (vía `Request::raw()`, accesor añadido al crate
//! `kernel`), replicando el lenguaje de `bio_for_each_segment`. Cada
//! segmento se mapea con `kmap_local_page`/`kunmap_local` (helpers del
//! bindings). El store es global (un disco) bajo un `Mutex` (kmalloc): los
//! slots, los búferes scratch y la tabla hash de matches viven en el estado,
//! nunca en la pila.
//!
//! Limitaciones v1 (docs/SPEC-estiba.md §7):
//!   - Solo atiende segmentos que cubren EXACTAMENTE una página 4 KiB
//!     alineada (lógico=4096 ⇒ swap y `dd bs=4K` cumplen). Cualquier otra
//!     forma de segmento completa el request con error.
//!   - Lectura de slot no escrito → página de ceros.
//!   - Sin sysfs propio (el crate kernel 6.12 no expone kobject/sysfs):
//!     estadísticas por `pr_info!` al descargar + contadores genéricos de
//!     `/sys/block/estiba/stat`.

use kernel::{
    alloc::{flags, KBox, KVec},
    bindings,
    block::mq::{self, gen_disk, Operations, TagSet},
    error::{code, Result},
    new_mutex, pr_info,
    prelude::*,
    sync::{Arc, Mutex},
    types::ARef,
};

#[path = "gen_codec/lib.rs"]
mod codec;

use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};

/// Geometría del disco (512 B/sector; slot de 4 KiB).
const DISK_SIZE: u64 = 4 << 20; // 4 MiB
const SLOTS: usize = (DISK_SIZE / 4096) as usize;
const SLOT_SHIFT: u64 = 3; // sector >> 3 = slot de 4 KiB
const SIZE_4K: usize = 4096;
const SCRATCH_LEN: usize = codec::compress_bound(SIZE_4K);

/// Bits de operación del `request` (REQ_OP_BITS=8, include/linux/blk_types.h).
const REQ_OP_MASK: u32 = 0xFF;
const REQ_OP_READ: u32 = 0;
const REQ_OP_WRITE: u32 = 1;
const REQ_OP_FLUSH: u32 = 2;

/// BLK_STS_IOERR = 10 (blk_types.h); el bindings solo genera BLK_STS_OK.
const BLK_STS_IOERR: u8 = 10;

/// Cabecera del slot (codec): flag RAW bit0 del byte 3.
const FLAG_RAW_BIT: u8 = 1;

// ---------------------------------------------------------------------------
// Estado global del dispositivo (1 disco v1).
//
// El kernel 6.12 NO expone `OnceLock` ni locks con constructor `const`, así
// que el store se asigna una vez en `init` (con `KBox`) y se filtra
// (`KBox::leak`) a un `AtomicPtr`. El acceso desde `queue_rq` se serializa con
// un spinlock propio: `queue_rq` corre en contexto donde NO se puede dormir,
// así que ni `Mutex` ni `GFP_KERNEL` son válidos aquí.
// ---------------------------------------------------------------------------

static STORE: AtomicPtr<EstibaStore> = AtomicPtr::new(core::ptr::null_mut());
static STORE_LOCK: AtomicBool = AtomicBool::new(false);

/// Guard del spinlock del store (libera en `Drop`).
struct StoreGuard;

impl StoreGuard {
    fn acquire() -> Self {
        while STORE_LOCK
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            core::hint::spin_loop();
        }
        StoreGuard
    }
}

impl Drop for StoreGuard {
    fn drop(&mut self) {
        STORE_LOCK.store(false, Ordering::Release);
    }
}

struct EstibaStore {
    /// slot i = payload del slot (o `None` = página de ceros).
    slots: KVec<Option<KVec<u8>>>,
    /// búfer destino de `compress_into` (separado de `scratch`: el codec
    /// exige dst≠scratch).
    bcomp: KVec<u8>,
    /// búfer scratch del codec (tokens + sección de entropía).
    scratch: KVec<u8>,
    /// tabla hash de la capa LZ (la resetea el propio codec por llamada).
    ht: KVec<u32>,
}

impl EstibaStore {
    fn new() -> Result<Self> {
        let mut slots =
            KVec::with_capacity(SLOTS, flags::GFP_KERNEL).map_err(|_| code::ENOMEM)?;
        for _ in 0..SLOTS {
            slots.push(None, flags::GFP_KERNEL).map_err(|_| code::ENOMEM)?;
        }
        let bcomp =
            KVec::from_elem(0u8, SCRATCH_LEN, flags::GFP_KERNEL).map_err(|_| code::ENOMEM)?;
        let scratch =
            KVec::from_elem(0u8, SCRATCH_LEN, flags::GFP_KERNEL).map_err(|_| code::ENOMEM)?;
        let ht = KVec::from_elem(0u32, codec::HASH_TAB_LEN, flags::GFP_KERNEL)
            .map_err(|_| code::ENOMEM)?;
        Ok(Self { slots, bcomp, scratch, ht })
    }
}

/// Contadores acumulados (volcados por `pr_info!` en el unload).
struct EstibaStats {
    reads: AtomicU64,
    writes: AtomicU64,
    read_bytes: AtomicU64,
    write_bytes: AtomicU64,
    compressed: AtomicU64,
    raw: AtomicU64,
    errors: AtomicU64,
}

static STATS: EstibaStats = EstibaStats {
    reads: AtomicU64::new(0),
    writes: AtomicU64::new(0),
    read_bytes: AtomicU64::new(0),
    write_bytes: AtomicU64::new(0),
    compressed: AtomicU64::new(0),
    raw: AtomicU64::new(0),
    errors: AtomicU64::new(0),
};

// ---------------------------------------------------------------------------
// Módulo
// ---------------------------------------------------------------------------

module! {
    type: EstibaModule,
    name: "estiba",
    author: "estiba",
    description: "estiba - compressed RAM block device (Rust)",
    license: "GPL v2",
}

struct EstibaModule {
    _disk: Pin<KBox<Mutex<gen_disk::GenDisk<EstibaBlock>>>>,
}

impl kernel::Module for EstibaModule {
    fn init(_module: &'static ThisModule) -> Result<Self> {
        pr_info!("estiba: loading {SLOTS} slots of 4 KiB ({} MiB logical)\n", DISK_SIZE >> 20);

        let tagset = Arc::pin_init(TagSet::new(1, 256, 1), flags::GFP_KERNEL)?;

        let disk = gen_disk::GenDiskBuilder::new()
            .capacity_sectors(DISK_SIZE >> 9)
            .logical_block_size(4096)?
            .physical_block_size(4096)?
            .rotational(false)
            .build(format_args!("estiba{}", 0), tagset)?;

        let disk = KBox::pin_init(new_mutex!(disk, "estiba:disk"), flags::GFP_KERNEL)?;

        let store = KBox::new(EstibaStore::new()?, flags::GFP_KERNEL)?;
        STORE.store(KBox::leak(store) as *mut EstibaStore, Ordering::Release);

        Ok(Self { _disk: disk })
    }
}

impl Drop for EstibaModule {
    fn drop(&mut self) {
        pr_info!(
            "estiba: unloaded: reads={} writes={} bytes={}+{} comp={} raw={} errors={}\n",
            STATS.reads.load(Ordering::Relaxed),
            STATS.writes.load(Ordering::Relaxed),
            STATS.read_bytes.load(Ordering::Relaxed),
            STATS.write_bytes.load(Ordering::Relaxed),
            STATS.compressed.load(Ordering::Relaxed),
            STATS.raw.load(Ordering::Relaxed),
            STATS.errors.load(Ordering::Relaxed),
        );
    }
}

// ---------------------------------------------------------------------------
// Dispositivo de bloque
// ---------------------------------------------------------------------------

struct EstibaBlock;

#[vtable]
impl Operations for EstibaBlock {
    fn queue_rq(rq: ARef<mq::Request<Self>>, _is_last: bool) -> Result {
        // El store nunca falta (init lo crea antes de registrar el disco); si
        // un request llegara antes, lo completamos vacío en vez de colgarlo.
        let ptr = STORE.load(Ordering::Acquire);
        if ptr.is_null() {
            let _ = mq::Request::end_ok(rq);
            return Ok(());
        }
        // Serializa el acceso al store (queue_rq no puede dormir).
        let guard = StoreGuard::acquire();
        // SAFETY: `ptr` apunta al `EstibaStore` filtrado en `init`, vivo
        // durante toda la vida del módulo; el spinlock garantiza acceso
        // exclusivo y el `guard` lo libera al final de la función.
        let st = unsafe { &mut *ptr };

        let mut ioerr = false;

        // SAFETY (bloque entero): `rq.raw()` devuelve un puntero válido a un
        // `struct request` cuya cadena de bios y páginas están vivas mientras
        // el request esté en vuelo. queue_rq corre en el CPU de despacho del
        // tagset y completamos siempre antes de volver, así que los pares
        // kmap_local_page/kunmap_local ocurren en el mismo CPU y ninguna
        // referencia sobrevive a la completión.
        unsafe {
            let rq_raw = rq.raw();
            let op = (*rq_raw).cmd_flags & REQ_OP_MASK;

            if op == REQ_OP_FLUSH {
                // Sin datos: no-op con éxito (v1).
            } else if op == REQ_OP_READ || op == REQ_OP_WRITE {
                let is_read = op == REQ_OP_READ;
                let mut bio = (*rq_raw).bio;

                while !bio.is_null() && !ioerr {
                    // Acto de `bio_for_each_segment` (paso único de bios).
                    let bi_opf = (*bio).bi_opf & REQ_OP_MASK;
                    if bi_opf != op {
                        ioerr = true;
                        break;
                    }

                    // --- iterador de segmentos (bvec_iter +
                    // bvec_iter_advance_single) ---
                    let mut it = (*bio).bi_iter;
                    while it.bi_size != 0 && !ioerr {
                        // SAFETY: bi_io_vec tiene bi_vcnt entradas (o más) y
                        // bi_idx < bi_vcnt lo mantiene el iterador.
                        let bv = &*(*bio).bi_io_vec.add(it.bi_idx as usize);
                        let done = it.bi_bvec_done as usize;
                        let bv_len = bv.bv_len as usize;
                        // Off en bytes dentro del bvec. Un `bio_vec` puede
                        // cubrir VARIAS páginas (folios grandes): la página a
                        // mapear es la `off/PAGE_SIZE`-ésima desde `bv_page`,
                        // no siempre `bv_page`.
                        let off = bv.bv_offset as usize + done;
                        let mut len = it.bi_size as usize;
                        len = core::cmp::min(len, bv_len - done);
                        len = core::cmp::min(len, SIZE_4K - off % SIZE_4K);

                        // v1: solo páginas completas alineadas (swap/dd bs=4K).
                        if len != SIZE_4K || off % SIZE_4K != 0 {
                            ioerr = true;
                            break;
                        }
                        // El slot se deriva del sector del PROPIO `bvec_iter`
                        // (no de `rq->__sector`): los bios de lectura pueden
                        // empezar con `bi_idx != 0` (bios partidos), y solo
                        // `bi_iter.bi_sector` refleja el sector real del
                        // segmento actual. Se avanza junto con el iterador, como
                        // `bvec_iter_advance_single`.
                        let sec: u64 = it.bi_sector;
                        let slot = (sec >> SLOT_SHIFT) as usize;
                        if slot >= SLOTS {
                            ioerr = true;
                            break;
                        }

                        // SAFETY: `bv_page + off/4096` pertenece al segmento;
                        // kmap_local_page la mapea 1:1 en este CPU.
                        let va = bindings::kmap_local_page(bv.bv_page.add(off >> 12));
                        if va.is_null() {
                            ioerr = true;
                            break;
                        }

                        if is_read {
                            match st.slots[slot].as_ref() {
                                None => {
                                    // SAFETY: `va` es una página mapeada 1:1.
                                    core::ptr::write_bytes(va as *mut u8, 0, SIZE_4K);
                                }
                                Some(slot_data) => {
                                    // SAFETY: íd., página completa de destino.
                                    let dst =
                                        core::slice::from_raw_parts_mut(va.cast::<u8>(), SIZE_4K);
                                    match codec::decompress_into(slot_data, dst, &mut st.scratch) {
                                        Ok(n) if n == SIZE_4K => {}
                                        _ => ioerr = true,
                                    }
                                }
                            }
                            STATS.read_bytes.fetch_add(SIZE_4K as u64, Ordering::Relaxed);
                            STATS.reads.fetch_add(1, Ordering::Relaxed);
                        } else {
                            // SAFETY: `va` es la página de origen (bio), completa
                            // y alineada.
                            let src = core::slice::from_raw_parts(va.cast::<u8>(), SIZE_4K);
                            match codec::compress_into(src, &mut st.bcomp, &mut st.scratch, &mut st.ht)
                            {
                                Ok(n) => {
                                    // Raw si el codec no mejoró (flag bit0 del
                                    // header; FLAG_RAW del codec es pub(crate)).
                                    if st.bcomp[3] & FLAG_RAW_BIT != 0 {
                                        STATS.raw.fetch_add(1, Ordering::Relaxed);
                                    } else {
                                        STATS.compressed.fetch_add(1, Ordering::Relaxed);
                                    }
                                    // Sin `?`: queue_rq debe completar SIEMPRE
                                    // el request antes de volver. GFP_ATOMIC:
                                    // queue_rq no puede dormir.
                                    let mut newv: KVec<u8> = KVec::new();
                                    if newv
                                        .extend_from_slice(&st.bcomp[..n], flags::GFP_ATOMIC)
                                        .is_err()
                                    {
                                        ioerr = true;
                                    } else {
                                        st.slots[slot] = Some(newv);
                                    }
                                }
                                Err(_) => ioerr = true,
                            }
                            STATS.write_bytes.fetch_add(SIZE_4K as u64, Ordering::Relaxed);
                            STATS.writes.fetch_add(1, Ordering::Relaxed);
                        }

                        // SAFETY: desmapea la página de este segmento (mismo
                        // CPU que la mapeó).
                        bindings::kunmap_local(va);
                        if ioerr {
                            break;
                        }

                        // --- advance (mirror de bvec_iter_advance_single) ---
                        let mut done_after = done + len;
                        if done_after == bv_len {
                            done_after = 0;
                            it.bi_idx += 1;
                        }
                        it.bi_bvec_done = done_after as u32;
                        it.bi_size -= len as u32;
                        it.bi_sector += (len as u64) >> 9; // 4096/512
                    }

                    // SAFETY: bi_next, cadena viva del request.
                    bio = (*bio).bi_next;
                }
            } else {
                // Operaciones no soportadas (DISCARD, WRITE_ZEROES, ...).
                ioerr = true;
            }
        }

        // Libera el spinlock ANTES de completar (la completión puede
        // reencolar y no debe re-entrar con el lock tomado).
        drop(guard);

        if ioerr {
            STATS.errors.fetch_add(1, Ordering::Relaxed);
            // SAFETY: `rq.raw()` sigue apuntando a un request válido con el
            // ARef vivo; bloquear la completión Y soltar el ARef (refcount) es
            // el cierre correcto (NUNCA llamar después a end_ok: doble
            // completión → UAF).
            let rptr = unsafe { rq.raw() };
            // SAFETY: íd.; el request se completa con error y el bloque dueño
            // le hace el último put vía el slot retirado del tagset.
            unsafe { bindings::blk_mq_end_request(rptr, BLK_STS_IOERR as bindings::blk_status_t) };
            drop(rq);
        } else {
            let _ = mq::Request::end_ok(rq);
        }
        Ok(())
    }

    fn commit_rqs() {}
}
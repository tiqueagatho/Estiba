# estiba — módulo Rust OUT-OF-TREE de dispositivo de bloque comprimido en RAM.
#
# kbuild 6.12 solo compila UN archivo .rs por módulo OOT; el codec (crate
# no_std multi-archivo con una sola alocación de producción) se vendorea en
# gen_codec/ mediante gen-codec.sh (regenerado en cada build). estiba.rs lo
# incluye con `#[path = "gen_codec/lib.rs"] mod codec;`.

obj-m := estiba.o

KDIR ?= /usr/src/linux-source-6.12

all: gen
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules

.PHONY: gen
gen:
	./gen-codec.sh

.PHONY: clean
clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) clean
	rm -rf gen_codec

.PHONY: parity
parity: gen
	cd parity && cargo test
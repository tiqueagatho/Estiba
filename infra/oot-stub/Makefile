# Kbuild externo (Fase 0): valida que el árbol 6.12 compila módulos Rust
# OUT-OF-TREE. Si `insmod` pasa en la VM del sandbox, tram.ko (Fase 3)
# es viable como módulo externo.
obj-m += tram_oot_stub.o
KDIR ?= /tree

all:
	$(MAKE) -C $(KDIR) M=$(CURDIR) modules

clean:
	$(MAKE) -C $(KDIR) M=$(CURDIR) clean

.PHONY: all clean

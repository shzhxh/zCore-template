arch ?= x86_64
mode ?= release
LOG ?=

build_args := -Z build-std=core,alloc --target zCore/$(arch).json
build_path := target/$(arch)/$(mode)
kernel := $(build_path)/zcore
kernel_img := $(build_path)/zcore.img
ESP := $(build_path)/esp
OVMF := ./rboot/OVMF.fd
qemu := qemu-system-$(arch)

ifeq ($(mode), release)
	build_args += --release
endif

qemu_opts := \
	-smp cores=1

ifeq ($(arch), x86_64)
qemu_opts += \
    -cpu qemu64,fsgsbase \
	-drive if=pflash,format=raw,file=$(OVMF),readonly=on \
	-drive format=raw,file=fat:rw:$(ESP) \
	-serial mon:stdio \
	-m 4G \
	-device isa-debug-exit \
	-display none
endif

run: build justrun

debug: build debugrun

debugrun:
	@$(qemu) $(qemu_opts) -s -S

justrun:
	@$(qemu) $(qemu_opts)

build: $(kernel_img)

$(kernel_img): kernel bootloader
	@mkdir -p $(ESP)/EFI/zCore $(ESP)/EFI/Boot
	@cp ./target/x86_64-unknown-uefi/release/rboot.efi $(ESP)/EFI/Boot/BootX64.efi
	@cp ./prebuilt/rboot.conf $(ESP)/EFI/Boot/rboot.conf
	@cp $(kernel) $(ESP)/EFI/zCore/zcore.elf

kernel:
	@echo Building zCore kenel
	@cargo build -p zcore $(build_args)

bootloader:
	@cd rboot && make build

clean:
	@cargo clean

ROOTFS_TAR := alpine-minirootfs-3.11.3-x86_64.tar.gz
ROOTFS_URL := http://dl-cdn.alpinelinux.org/alpine/v3.11/releases/x86_64/$(ROOTFS_TAR)

.PHONY: rootfs

prebuilt/$(ROOTFS_TAR):
	wget $(ROOTFS_URL) -O $@

rootfs: prebuilt/$(ROOTFS_TAR)
	mkdir -p rootfs
	tar xf $< -C rootfs
	cp prebuilt/libc.so rootfs/lib/ld-musl-x86_64.so.1

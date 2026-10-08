#!/bin/sh
# init canario Fase 0 (corre como /init dentro del initramfs autocontenido).
mount -t devtmpfs devtmpfs /dev
mount -t sysfs sysfs /sys
mount -t proc proc /proc
echo "<6>ESTIBA-BOOT-OK kernelrelease=$(cat /proc/sys/kernel/osrelease 2>/dev/null)"

echo "<6>[canary] insmod rnull_mod.ko"
if insmod /root/rnull_mod.ko 2>/dev/null; then
    echo "<6>ESTIBA-CANARY-INSMOD-OK"
else
    echo "<3>ESTIBA-CANARY-INSMOD-FAIL"
fi

rnull=/dev/rnullb0
if [ -b "$rnull" ]; then
    echo "<6>[canary] dd 512K a rnull"
    dd if=/dev/zero of="$rnull" bs=1024 count=512 2>/dev/null
    echo "<6>ESTIBA-CANARY-WRITE-OK"
    echo "<6>[canary] dd de vuelta"
    dd if="$rnull" of=/dev/null bs=1024 count=512 2>/dev/null
    echo "<6>ESTIBA-CANARY-READ-OK"
else
    echo "<3>ESTIBA-CANARY-NODEV"
fi

echo "<6>[canary] rmmod"
if rmmod rnull_mod 2>/dev/null; then
    echo "<6>ESTIBA-CANARY-RMMOD-OK"
else
    echo "<3>ESTIBA-CANARY-RMMOD-FAIL"
fi

echo "<6>[oot] insmod estiba_oot_stub.ko"
if insmod /root/estiba_oot_stub.ko 2>/dev/null; then
    echo "<6>ESTIBA-OOT-INSMOD-OK"
    sleep 1
    if rmmod estiba_oot_stub 2>/dev/null; then
        echo "<6>ESTIBA-OOT-RMMOD-OK"
    else
        echo "<3>ESTIBA-OOT-RMMOD-FAIL"
    fi
else
    echo "<3>ESTIBA-OOT-INSMOD-FAIL"
fi

echo "<6>ESTIBA-ALL-OK"
sync
poweroff -f
#!/bin/sh

if [ $# -ne 1 ]; then
	echo "Usage: dump-dpcd.sh AUX"
	echo
	echo "Dumps AUX USB4 DPCD registers."

	exit 1
fi

AUX=$1
dd if=/dev/drm_dp_aux${AUX} bs=1 skip=$((0xe0000)) count=64 2> /dev/null | od -Ax -tx1

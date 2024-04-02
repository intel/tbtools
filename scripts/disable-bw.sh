#!/bin/sh

if [ $# -ne 1 ]; then
	echo "Usage: disable-bw.sh AUX"
	echo
	echo "Disables USB4 bandwidth allocation mode from DPCD side."

	exit 1
fi

AUX=$1
VAL=0x00
OVAL=$(printf "\%o" $VAL)
printf "%b" "$OVAL" |
	dd of=/dev/drm_dp_aux${AUX} bs=1 count=1 seek=$((0xe0030)) 2> /dev/null
echo "BW allocation mode disabled"

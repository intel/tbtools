#!/bin/sh -e

if [ $# -lt 1 ]; then
	echo "Usage: estimated-bw.sh AUX"
	echo
	echo "Dumps estimated bandwidth (in Mbit/s) through USB4 DPCD registers."

	exit 1
fi

AUX=$1
GR=$(dd if=/dev/drm_dp_aux${AUX} bs=1 skip=$((0xe0022)) count=1 2> /dev/null |
	od -An -tx1 |
	sed 's/^[ \t]*//')
case $GR in
	00)
		GR=250
		;;
	01)
		GR=500
		;;
	02)
		GR=1000
		;;
	*)
		echo "Error: unsupported granularity: $GR" 1>&2
		exit 1
		;;
esac
EST=$(dd if=/dev/drm_dp_aux${AUX} bs=1 skip=$((0xe0023)) count=1 2> /dev/null |
	od -An -tx1 |
	sed 's/^[ \t]*//')
BW=$((0x$EST * $GR))
printf "Estimated BW: %d Mb/s\n" $BW

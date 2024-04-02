#!/bin/sh -e

if [ $# -lt 2 ]; then
	echo "Usage: alloc-bw.sh AUX BANDWIDTH"
	echo
	echo "Allocates BANDWIDTH (in Mbit/s) through USB4 DPCD registers."

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
BW=$(($2 / $GR))
OBW=$(printf "\%o" $BW)
printf "%b" "$OBW" |
	dd of=/dev/drm_dp_aux${AUX} bs=1 count=1 seek=$((0xe0031)) 2> /dev/null

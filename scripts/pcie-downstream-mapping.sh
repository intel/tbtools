#!/bin/bash
#
# Runs Get PCIe Downstream Mapping router operation and returns the
# results.
#

set -e

GET_PCIE_DOWNSTREAM_MAPPING=0x30

usage() {
	echo "Usage: $0 [DOMAIN] ROUTE"
	echo
	echo "Runs Get PCIe Downstream Mapping router operation"
	exit 1
}

if [ $# -lt 1 ]; then
	usage
	exit 1
fi

if [ $# -eq 1 ]; then
	domain=0
	route=$1
else
	domain=$1
	route=$2
fi

#
# pcie_read_one_mapping() - Reads one PCIe mapping entry
# @domain: Domain number
# @route: Route string of the router
#
# As long as there are entries left returns 0. When final entry is
# encountered returns 1.
#
pcie_read_one_mapping() {
	local domain=$1
	local route=$2
	local metadata

	tbset -d $domain -r $route 				 \
		ROUTER_CS_26.Opcode=$GET_PCIE_DOWNSTREAM_MAPPING \
		ROUTER_CS_26.OV=1

	metadata=$(tbget -d $domain -r $route ROUTER_CS_25)
	data=($(tbdump -d $domain -r $route -N 2 ROUTER_CS_9))
	entries=$((metadata & 0xff))
	index=$(((metadata & 0xff00) >> 8))

	native=$((data[0] & 0x1))
	rid=$((data[1] & 0xffff))
	nfpb_bus=$((rid >> 8))
	nfpb_dev=$(((rid >> 3) & 0x1f))
	nfpb_fn=$((rid & 0x7))
	rid=$(((data[1] & 0xffff0000) >> 16))
	fpb_bus=$((rid >> 8))
	fpb_dev=$(((rid >> 3) & 0x1f))
	fpb_fn=$((rid & 0x7))

	if (( $native == 1 )); then
		printf "$index: Native FPB: %02x:%02x.%x Non-FPB: %02x:%02x.%x\n" \
			$fpb_bus $fpb_dev $fpb_fn $nfpb_bus $nfpb_dev $nfpd_fn
	else
		adapter=$(((data[0] & 0x7e) >> 1))
		printf "$index: Tunneled $adapter FPB: %02x:%02x.%x Non-FPB: %02x:%02x.%x\n" \
			$fpb_bus $fpb_dev $fpb_fn $nfpb_bus $nfpb_dev $nfpd_fn
	fi

	if (( $index == $entries - 1 )); then
		return 1
	fi

	return 0
}

while pcie_read_one_mapping $domain $route; do
	:;
done

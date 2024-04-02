#!/bin/sh -e
#
# Dumps Intel USB4 router NVM image version. Can be useful at least with
# the integrated hosts that do not expose all NVM operations.
#

# NVM Read router operation
NVM_READ=0x22

usage() {
	echo "Usage: $0 [DOMAIN] ROUTE"
	echo
	echo "Reads Intel USB4 router NVM image version"
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
# nvm_read() - Reads up to 64-bytes from router NVM
# @domain: Domain number
# @route: Route string of the router
# @address: Double word address in the NVM
# @len: Number of double words to read (max 16)
#
nvm_read() {
	local domain=$1
	local route=$2
	local address=$3
	local len=$4

	local metadata=$(printf "0x%08x" $((len << 24 | address << 2)))

	tbset -d $domain -r $route ROUTER_CS_25=$metadata
	tbset -d $domain -r $route ROUTER_CS_26.Opcode=$NVM_READ ROUTER_CS_26.OV=1

	tbdump -d $domain -r $route -N $len ROUTER_CS_9
}

value=$(nvm_read $domain $route 2 1)

printf "Thunderbolt binary version: %x.%x\n" \
	$(((value >> 16) & 0xff)) $(((value >> 8) & 0xff))

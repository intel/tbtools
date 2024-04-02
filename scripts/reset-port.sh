#!/bin/sh -e
#
# Triggers Downstream Port Reset on an adapter. Can be useful for
# simulating hotplug for instance.
#

usage() {
	echo "Usage: $0 [DOMAIN] ROUTE ADAPTER"
	echo
	echo "Triggers Downstream Port Reset on adapter"
	exit 1
}

if [ $# -lt 2 ]; then
	usage
	exit 1
fi

if [ $# -eq 2 ]; then
	domain=0
	route=$1
	adapter=$2
else
	domain=$1
	route=$2
	adapter=$3
fi

#
# reset_port() - Issue downstream reset on USB4 port
# @domain: Domain number
# @router: Route string of the router
# @adapter: Lane adapter number
#
reset_port() {
	local domain=$1
	local route=$2
	local adapter=$3
	local val

	# Check type first
	val=$(tbget -d $domain -r $route -a $adapter ADP_CS_2)
	val=$(printf "0x%x" $((val & 0xffffff)))
	case $val in
	0x1)
		;;
	*)
		echo "Error: unsupported adapter type: $val" 1>&2
		exit 1
		;;
	esac

	tbset -d $domain -r $route -a $adapter PORT_CS_19.DPR=1
	sleep 1
	tbset -d $domain -r $route -a $adapter PORT_CS_19.DPR=0

	printf "Domain $domain Route $route Adapter $adapter: reset done\n"
}

reset_port $domain $route $adapter

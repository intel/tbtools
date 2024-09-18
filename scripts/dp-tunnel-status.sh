#!/bin/bash -e
#
# Dumps the DisplayPort tunnel status.
#

usage() {
	echo "Usage: $0 [DOMAIN] ROUTE ADAPTER"
	echo
	echo "Dumps DP tunnel status"
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
# dp_tunnel_status() - Reads and dumps DisplayPort tunnel status
# @domain: Domain number
# @router: Route string of the router
# @adapter: DisplayPort IN adapter
#
dp_tunnel_status() {
	local domain=$1
	local route=$2
	local adapter=$3
	local val

	# Check type first
	val=$(tbadapters -d $domain -r $route -a $adapter -S | sed 1d | cut -d, -f 2)
	if [[ $val != "DisplayPort IN" ]]; then
		echo "Error: DisplayPort IN adapter expected" 1>&2
		exit 1
	fi

	# Check if paths are enabled
	val=$(tbget -d $domain -r $route -a $adapter ADP_DP_CS_0.VE)
	if [ $val != "0x1" ]; then
		printf "No active tunnel\n"
		return
	fi

	# Is BW alloc mode enabled?
	val=$(tbget -d $domain -r $route -a $adapter ADP_DP_CS_8.DPME)
	if [ $val = "0x1" ]; then
		read -r granularity estimated allocated requested group <<EOF
$(tbget -d $domain -r $route -a $adapter ADP_DP_CS_2.GR \
					 "ADP_DP_CS_2.Estimated BW" \
					 "DP_STATUS.Allocated BW" \
					 "ADP_DP_CS_8.Requested BW" \
					 "ADP_DP_CS_2.Group_ID" |
					 xargs)
EOF
		case $granularity in
			0x0)
				granularity=250
				;;
			0x1)
				granularity=500
				;;
			0x2)
				granularity=1000
				;;
			*)
				echo "Error: unsupported granularity: $granularity" 1>&2
				exit 1
				;;
		esac

		printf "Group       : $group\n"
		rate=$((granularity * estimated))
		printf "Estimated   : $rate Mb/s\n"
		rate=$((granularity * requested))
		printf "Requested   : $rate Mb/s\n"
		rate=$((granularity * allocated))
		printf "Allocated   : $rate Mb/s\n"
	else
		read -r lanes rate <<EOF
$(tbget -D -d $domain -r $route -a $adapter "DP_STATUS.Lane Count" \
					 "DP_STATUS.Link Rate" |
					 xargs)
EOF
		case $rate in
			0)
				rate=1620
				;;
			1)
				rate=2700
				;;
			2)
				rate=5400
				;;
			3)
				rate=8100
				;;
			*)
				echo "Error: unsupported rate: $rate" 1>&2
				exit 1
				;;
		esac


		printf "Rate        : $rate Mb/s x $lanes\n"
	fi

	printf "Capabilities:\n"
	tbdump -d $domain -r $route -a $adapter -N 1 -vv DP_COMMON_CAP
}

dp_tunnel_status $domain $route $adapter

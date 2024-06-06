#!/bin/bash

#
# dp_tunnel_status() - Reads DisplayPort tunnel status
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
	val=$(tbget -d $domain -r $route -a $adapter ADP_CS_2)
	val=$(printf "0x%x" $((val & 0xffffff)))
	case $val in
	0xe0101 | 0xe0102)
		;;
	*)
		echo "Error: unsupported adapter type: $val" 1>&2
		exit 1
		;;
	esac

	# Check if paths are enabled
	val=$(tbget -d $domain -r $route -a $adapter ADP_DP_CS_0.VE)
	if [ $val != "0x1" ]; then
		printf "\t\tTunnel    : inactive\n"
		return
	else
		printf "\t\tTunnel    : active\n"
	fi

	# read link rate and line count
	read -r lanes lrate <<EOF
$(tbget -D -d $domain -r $route -a $adapter "DP_STATUS.Lane Count" \
						 "DP_STATUS.Link Rate" |  \
						xargs)
EOF
	case $lrate in
		0)
			lrate=1620
			hbr="RBR"
			;;
		1)
			lrate=2700
			hbr="HBR"
			;;
		2)
			lrate=5400
			hbr="HBR2"
			;;
		3)
			lrate=8100
			hbr="HBR3"
			;;
		*)
			echo "Error: unsupported rate: $lrate" 1>&2
			exit 1
			;;
	esac

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
		printf "\t\tDPBWMode  : enabled\n"
		printf "\t\tGroup     : $group\n"
		rate=$((granularity * estimated))
		rate_gbps=$(awk "BEGIN {printf \"%.2f\", $rate / 1000}")
		printf "\t\tEstimated : $rate_gbps Gb/s\n"
		rate=$((granularity * requested))
		rate_gbps=$(awk "BEGIN {printf \"%.2f\", $rate / 1000}")
		printf "\t\tRequested : $rate_gbps Gb/s\n"
		rate=$((granularity * allocated))
		rate_gbps=$(awk "BEGIN {printf \"%.2f\", $rate / 1000}")
		printf "\t\tAllocated : $rate_gbps Gb/s\n"
	else
		printf "\t\tDPBWMode  : disabled\n"
		allocated_bandwidth=$(awk "BEGIN {printf \"%.2f\", $lrate * $lanes * \
								8 / 10}")
		allocated_bandwidth_gbps=$(awk "BEGIN {printf \"%.2f\",             \
									$allocated_bandwidth / 1000}")
		printf "\t\tAllocated : $allocated_bandwidth_gbps Gb/s\n"
	fi
	rate_gbps=$(awk "BEGIN {printf \"%.2f\", $lrate / 1000}")
	printf "\t\tLink Rate : $hbr($rate_gbps Gb/s)\n"
	printf "\t\tLane Count: $lanes\n"
}

printf "[Thunderbolt/USB4 Info]:\n"
router=0
num_domains=$(tblist -SA | grep  Domain | wc -l)
for domain in $(seq 0 $((num_domains-1))); do
	printf "TBHost$domain:\n"
	while read -r adapter; do
		printf "\tAdapter:DP IN-$adapter\n"
		dp_tunnel_status $domain $router $adapter
	done < <(tbadapters --route 0 | grep "DisplayPort IN" | awk '{sub(/:/,"");
				 print $1}')
done
echo
printf "[Display Info]:\n"
# Path to the i915_display_info file
DISPLAY_INFO_PATH="/sys/kernel/debug/dri/0/i915_display_info"

# Check if the file exists
if [ ! -f "$DISPLAY_INFO_PATH" ]; then
	echo "$DISPLAY_INFO_PATH not found!"
	exit 1
fi

# Find all unique pipe identifiers
pipes=$(awk -F '[][]' '/\[CRTC:[0-9]+:pipe [A-Z]\]:/ { split($2, a, ":");
		 print a[3] }' "$DISPLAY_INFO_PATH" | sort -u)

# Find all connectors that are "connected", excluding eDP connectors
connected_connectors=$(grep -E " connected" "$DISPLAY_INFO_PATH" | \
						grep -v "eDP" | awk -F '[][]' '{print $2}')

# Loop through each pipe identifier and extract its information
active_pipe=0
for pipe in $pipes; do
	crtc_id=$(awk -F '[][]' "/\[CRTC:[0-9]+:pipe $pipe\]:/ {        \
				split(\$2, a, \":\"); print a[2] }"                 \
				"$DISPLAY_INFO_PATH" | head -n 1)
	if [ -z "$crtc_id" ]; then
		continue
	fi
	PATTERN="\[CRTC:$crtc_id:pipe $pipe\]:"
	pipe_info=$(awk -v pattern="$PATTERN" '
		$0 ~ pattern { printing = 1; next }
		printing && /^\t/ { print; next }
		printing && !/^\t/ { exit }
		' "$DISPLAY_INFO_PATH")
	for connector in $connected_connectors; do
		if echo "$pipe_info" | grep -q "$connector"; then
			printf "\t$connector: connected\n"
			active_pipe=1
			mode_value=$(echo "$pipe_info" | grep -o 'mode="[^"]*"' |
						awk -F '"' '{print $2}' | head -n 1)
			printf "\t\tResolution: $mode_value\n"
			bpp_value=$(echo "$pipe_info" | grep -o 'bpp=[^,]*' |
						awk -F '=' '{print $2}' | head -n 1)
			printf "\t\tBPP       : $bpp_value\n"
			freq_value=$(echo "$pipe_info" |
						grep -o -w 'mode="[^"]*":[[:space:]]*[0-9]*' |
						awk -F ':' '{print $2}' | tr -d '[:space:]' | head -n 1)
			printf "\t\tFrequency : ${freq_value}Hz\n"
			encoder_line=$(echo "$pipe_info" | grep "ENCODER")
			encoder_line_cleaned=$(echo "$encoder_line" |
								sed -e 's/[[:space:]]//g' -e 's/:connectors://')
			printf "\t\tEncoder   : $encoder_line_cleaned\n"
		fi
	done
done

if [ $active_pipe -eq 0 ]; then
	printf "\tNo active display detected!\n"
fi

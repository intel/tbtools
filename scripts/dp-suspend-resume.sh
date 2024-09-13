#!/bin/bash

set -ue

shopt -s extglob
shopt -s expand_aliases

CONFIG_FILE=.config

TBTEST_DIR=$(dirname $0)

. $TBTEST_DIR/utils.sh
. $TBTEST_DIR/type-utils.sh
. $TBTEST_DIR/tbtool-db.sh
. $TBTEST_DIR/tools.sh
. $TBTEST_DIR/trace.sh

# enable_debug /tmp/tbtest-debug.log

AUTORESUME_DELAY_SEC=2		# delay after a suspend until system is autoresumed
TEST_CYCLE_DELAY_SEC=30		# delay after resume until the next suspend/resume cycle
MAX_PING_DURATION_SEC=30        # maximum duration while waiting for a responsive network connection

MAX_TEST_CMD_RETRY_ATTEMPTS=10	# max number test commands are retried, in case they need this

MAX_REC_SCAN=300

FILTERED_DMESG_ERRORS="^done\.$"	# error/warning messages filtered out

declare -r ERR_STATS_CHANGED=243

declare -r TBT_DP_IN_ADAPTERS_CONF_NAME=TBT_DP_IN_ADAPTERS
TBT_DP_IN_ADAPTERS=()		# list of preconfigured  (D:R:A [D:R:A ...]) enabled DP IN
				# adapters to test. If unset the adapters will be detected.

[ -f "$CONFIG_FILE" ] && . "$CONFIG_FILE"

epoch_delta=0			# delta between epoch and time since script start

declare -ra required_tools=(
	TBGET TBADAPTERS TBLIST
	DMESG AWK SED RM MKTEMP GREP RTCWAKE LSMOD MODPROBE RMMOD
	DD HEAD TAIL TEE WC CAT PRINTF WC DATE STTY STDBUF
	RM RMDIR KILL
)
declare -ra optional_tools=(
	SUDO TPUT
	PING NMAP
	LOCALE
)

modules="thunderbolt i915"

declare -a dp_in_adapters

trace_filter_msg_config=$TRACE_FILTER_EXCLUDE
trace_filter_msg_regex=""

declare -r FILTER_DEV_NONE=0
declare -r FILTER_DEV_TB=$(bit 0)
declare -r FILTER_DEV_GFX=$(bit 1)
declare -r FILTER_DEV_ALL=$((FILTER_DEV_TB | FILTER_DEV_GFX))

trace_filter_dev_mask=$FILTER_DEV_NONE
trace_filter_dev_regex=""
gfx_dev_regex=""
tb_dev_regex=""

test_started=
dry_run=false
adapters_got_detected=false

skip_test=false
reload_network=false
wait_network_connection=false

declare -r NMAP_STATUS_UP_REGEX="\bStatus: Up\b"

ping_tool=
ping_options=

max_cycle_count=0			# Number of test cycles, until interrupted if 0

declare -r TEST_PHASE_PREPARE=0
declare -r TEST_PHASE_RESUMED=1
declare -r TEST_PHASE_SUSPENDING=2
declare -r TEST_PHASE_STOPPED=3

declare -A test_phase_label

declare -r HAS_COMMANDS=$(bit 1)
declare -r HAS_COUNTDOWN=$(bit 2)
declare -r HAS_PROGRESS=$(bit 3)

declare -Ar test_phase_flags=(
	[$TEST_PHASE_PREPARE]=$((HAS_COMMANDS | HAS_PROGRESS))
	[$TEST_PHASE_RESUMED]=$((HAS_COMMANDS | HAS_COUNTDOWN))
	[$TEST_PHASE_STOPPED]=$((HAS_COMMANDS | HAS_COUNTDOWN))
)

test_cycle=0
test_phase=$TEST_PHASE_PREPARE
test_phase_prev=
test_phase_start=0
test_phase_status_str=""
test_phase_commands_str=""

declare -r ES_LAST=0
declare -r ES_CURRENT=1
declare -r ES_TOTAL=2
declare -r ES_REPORTED=3

declare -a es_dmesg=(0 0 0 0)
declare -a es_dmesg_flt=(0 0 0 0)
declare -a es_dpme=(0 0 0 0)
declare -a es_other=(0 0 0 0)
declare -a es_dropped=(0 0 0 0)

# record tracker for kmsg in a given stage of processing
declare -r RT_SEEK=0
declare -r RT_FEED=1
declare -r RT_PARSE=2

# last seen kmsg cycle number/timestamp
declare -a rec_tracker_seq=(-1 -1 -1)
declare -a rec_tracker_timestamp=(-1 -1 -1)
# timestamp at which the processing is considered done
declare -a rec_tracker_target_timestamp=(-1 -1 -1)
# last time the cycle number has changed
declare -a rec_tracker_update_time=(-1 -1 -1)

declare -r REC_TRACKER_POLL_DURATION=5

# error summing flags
declare -r NO_DMESG_FLT=$(bit 0)
declare -r NO_DROPPED=$(bit 1)

cycle_delay_expires=0
cycle_delay_left=0

last_cycle_with_errors=0

declare -r CS_SEQ=0
declare -r CS_TIMESTAMP=1
declare -r CS_REC=2

declare -a current_cycle_read=(0 0 0)
declare -a current_cycle_checked=(0 0 0)
declare -a current_cycle_start=(-1 0 0)

declare -r error_line_fmt="  %-26s %-6s %-6s"

successive_sig_count=0

# Handle these sig gracefully, setting only a flag here and letting the
# test exit/cleanup happen - by the main thread checking this flag
# periodically or after a command got interrupted by this signal - in
# the normal main process thread context.
#
# Successive signal interrupts of the same type will be handled
# immediately, without waiting for the main process thread.
graceful_sigs="INT TERM"
abort_sigs="EXIT"
all_sigs="$graceful_sigs $abort_sigs"

declare -Ar sig_causes=(
	["INT"]="interrupted"
	["TERM"]="terminated"
	["EXIT"]="aborted"
)

declare -A non_descendant_pids=()

init_test_phase_labels()
{
	test_phase_label=(
		[$TEST_PHASE_PREPARE]="Prepare"
		[$TEST_PHASE_RESUMED]="Resumed"
		[$TEST_PHASE_SUSPENDING]="Suspending.."
	)
}

grep_no_err()
{
	local err

	$GREP "$@"
	err=$?

	[ $err -eq 1 ] && return 0

	return $err
}

init_suspend_resume_tools()
{
	init_tools required_tools false || return 1
	init_tools optional_tools true || return 1

	init_sudo || return 1

	export -f grep_no_err
	export GREP
}

setup_server_ping_tool()
{
	local ip_address
	local ip_host
	local ip_port
	local nmap_output

	ip_address=$(get_ssh_server_address) || return 1

	ip_host=${ip_address%:*}
	ip_port=${ip_address#*:}

	if [ -n "${NMAP+x}" ]; then
		ping_tool=$NMAP
		ping_options="--host-timeout 1 -oG - $ip_host"

		nmap_output=$($ping_tool $ping_options 2>&1)
		if [[ $? -ne 0 ]] || [[ ! "$nmap_output" =~ $NMAP_STATUS_UP_REGEX ]]; then
			ping_tool=""
			ping_options=""
		fi
	fi

	[ -n "$ping_tool" ] && return 0

	if [ -z "${PING+x}" ]; then
		pr_err "Neither nmap or ping is available"
		return 1
	fi

	ping_tool=$PING
	ping_options="-c 1 -w 1 $ip_host"

	ping_output=$($ping_tool $ping_options 2>&1)
	if [ $? -ne 0 ]; then
		pr_err "Couldn't ping the $ip_host address:\n$ping_output"
		return 1
	fi
}

setup_server_ping()
{
	setup_server_ping_tool && return 0

	err_exit "Couldn't setup a ping tool required for a responsive network connection"
}

parse_options()
{
	local options
	local long_options
	local parsed

	options="c:swdh"
	long_options="cycle-count:,reload-network-module,skip-test,wait-network-connection,dry-run,help"

	# Using getopt to store the parsed options and arguments into $parsed
	parsed=$(getopt --options=$options --longoptions=$long_options --name "$(basename $0)" -- "$@") || return 1

	# Checking for errors
	if [[ $? -ne 0 ]]; then
		exit 2
	fi

	# Setting the parsed options and arguments to $@
	eval set -- "$parsed"

	usg=\
"Usage: $(basename $0) [OPTIONS]
OPTIONS:
        -c, --cycle-count <count>        Number of test cycles. By default test until interrupted.
        --reload-network-module          rmmod/modprobe network module across suspend/resume.
        -s, --skip-test                  Perform only the initialization steps, skip the actual test.
        -w, --wait-network-connection    Wait for responsive network connection after each test cycle.
        -d, --dry-run                    Don't perform tests, used for development.
        -h, --help                       Print this help.
"

	while true; do
		case "$1" in
		-c|--cycle-count)
			valid_number "$2" || err_exit "Invalid test cycle count \"$2\"\n"
			[ $2 -gt 0 ] || err_exit "The test cycle count must be at least 1\n"
			max_cycle_count=$2

			shift
			;;
		-s|--skip-test)
			skip_test=true
			;;
		--reload-network-module)
			reload_network=true
			;;
		-w|--wait-network-connection)
			wait_network_connection=true
			;;
		-d|--dry-run)
			dry_run=true
			;;
		-h|--help)
			echo "$usg"
			exit
			;;
		--)
			shift
			break
			;;
		*)
			err_exit "Unexpected option \"$1\"\n"
			;;
		esac

		shift
	done

	if [ $# -gt 0 ]; then
		err_exit "unexpected arguments \"$@\"\n$usg"
	fi

	if $skip_test && [ $max_cycle_count -ne 0 ]; then
		err_exit "Can't specify both --skip-test and --cycle-count\n"
	fi

	if $reload_network; then
		if [[ ! -v NETWORK_MODULE ]]; then
			err_exit "The NETWORK_MODULE variable must be set to reload the network module\n"
		fi

		if ! $LSMOD | grep "^\<$NETWORK_MODULE\>" > /dev/null ; then
			err_exit "The \"$NETWORK_MODULE\" network module is not loaded\n"
		fi
	fi

	if $wait_network_connection; then
		if $skip_test; then
			err_exit "Can't specify both --skip-test and --wait-network-connection\n"
		fi

		setup_server_ping
	fi
}

init_test_start()
{
	local epoch=$("$DATE" +%s)
	local script_start=$SECONDS

	epoch_delta=$((epoch - script_start))

	test_started=$epoch
	test_phase_start=$test_started
}

get_epoch_seconds()
{
	echo "$((SECONDS + epoch_delta))"
}

get_dp_in_adapters()
{
	local err
	local adps

	dp_in_adapters=()

	log "$(get_prefix Init): Looking for DP IN adapters ...\n"

	adps=$(get_configured_dp_in_adapters)
	err=$?
	if [ $err -eq 0 ]; then
		eval "dp_in_adapters=($adps)"
		return 0
	fi

	[ $err -ne 2 ] && return 1

	adps=$(find_enabled_dp_in_adapters)
	err=$?
	[ $err -ne 0 ] && return 1

	eval "dp_in_adapters=($adps)"

	if [ ${#dp_in_adapters[@]} -eq 0 ]; then
		log "Couldn't find any enabled DP IN adapters to test\n"
		return 2
	fi

	adapters_got_detected=true
}

init_dp_in_adapters()
{
	local config_detect_str
	local adapter_desc

	$dry_run && return 0

	get_dp_in_adapters || return 0

	if $adapters_got_detected; then
		config_detect_str=detected
	else
		config_detect_str=configured
	fi

	log "$(get_prefix Init): Testing the following $config_detect_str DP IN adapters:\n"

	for adapter_desc in "${dp_in_adapters[@]}"; do
		local -A dr_adapter=()
		local dra

		dradapter_deserialize dr_adapter "$adapter_desc"
		dra=$(get_adapter_dra dr_adapter)
		log_cont "  DRA:$dra Dev:${dr_adapter[$DR_DEV]} Pci-ID:${dr_adapter[$DR_PCI_ID]}\n"
	done
}

init_filter_regex()
{
	local pattern

	while IFS= read -r pattern; do
		trace_filter_msg_regex+=${trace_filter_msg_regex:+|}
		trace_filter_msg_regex+=$pattern
	done < <(echo "$FILTERED_DMESG_ERRORS")
}

get_canonical_intel_vendor_id()
{
	local vendor=$1

	[ "$vendor" = "$INTEL_ALIAS_VENDOR_ID" ] && vendor=$INTEL_VENDOR_ID

	echo "$vendor"
}

check_pci_address_collision()
{
	local -nr __addresses=$1
	local -r address=$2
	local -r vendor=$3
	local -r device=$4

	if [ -n "${__addresses["$address"]+x}" -a \
		"${__addresses["$address"]-}" != "$vendor:$device" ]; then
		local ven_dev_str="$vendor:$device, ${__addresses["$address"]}"

		pr_err "Multiple PCI IDs for the same address $ven_dev_str\n"

		return 1
	fi

	return 0
}

get_tb_dev_addresses()
{
	local -A addresses=()
	local address
	local dev_list
	local dev

	dev_list=$(get_tb_devices) || return 1

	for dev in $dev_list; do
		local domain route pci_id

		while IFS=, read -r domain route pci_id; do
			local vendor device

			vendor=$(sanitize_pci_vendor_id "${pci_id#*:}")
			device=$(sanitize_pci_device_id "${pci_id%%:*}")

			address=$(get_pci_dev_address "$vendor" "$device") || continue

			if ! check_pci_address_collision addresses "$address" \
							 "$vendor" "$device"; then
				continue
			fi

			addresses["$address"]="$vendor:$device"
		done < <(echo "$dev")
	done

	for address in ${!addresses[@]}; do
		echo "$address-${addresses[$address]}"
	done

	return 0
}

get_gfx_dev_addresses()
{
	local -A addresses=()
	local address
	local dir

	for dir in $SYSFS_DRM_CLASS_DIR/card+([0-9]); do
		local vendor device

		vendor=$(sanitize_pci_vendor_id "$(cat $dir/device/vendor)")
		device=$(sanitize_pci_device_id "$(cat $dir/device/device)")

		address=$(get_pci_dev_address "$vendor" "$device")

		if ! check_pci_address_collision addresses "$address" \
						 "$vendor" "$device"; then
			continue
		fi

		addresses["$address"]="$vendor:$device"
	done

	for address in ${!addresses[@]}; do
		echo "$address-${addresses[$address]}"
	done
}

init_dev_regex()
{
	local tb_addresses=""
	local gfx_addresses=""
	local address_desc

	for address_desc in $(get_tb_dev_addresses); do
		local address=${address_desc%%-*}

		tb_addresses+=" $address_desc"
		tb_dev_regex="${tb_dev_regex}${tb_dev_regex:+|}\+pci:$address"
	done

	for address_desc in $(get_gfx_dev_addresses); do
		local address=${address_desc%%-*}

		gfx_addresses+=" $address_desc"
		gfx_dev_regex="${gfx_dev_regex}${gfx_dev_regex:+|}\+pci:$address"
	done

	log "$(get_prefix Init): Using the following device dmesg filters:\n"

	log_cont "  TBT devices:\n"
	for address_desc in $tb_addresses; do
		local address=${address_desc%%-*}
		local pci_ids=${address_desc#*-}

		log_cont "    $address: $pci_ids\n"
	done

	log_cont "  GFX devices:\n"
	for address_desc in $gfx_addresses; do
		local address=${address_desc%%-*}
		local pci_ids=${address_desc#*-}

		log_cont "    $address: $pci_ids\n"
	done
}

get_dpme()
{
	local -nr adapter=$1
	local dpme
	local err

	if $dry_run; then
		echo 0x1
		return 0
	fi

	dpme=$(test_cmd $SUDO $TBGET \
		-d "${adapter[$DR_DOMAIN_IDX]}" \
		-r "${adapter[$DR_ROUTE]}" \
		-a "${adapter[$ADP_ID]}" \
		ADP_DP_CS_8.DPME)
	err=$?

	if [ "$dpme" != 0x0 -a "$dpme" != 0x1 ]; then
		log_err "Invalid DPME state: \"$dpme\"\n"
		err=1
	fi

	echo "$dpme"

	return $err
}

check_dpme()
{
	local dr_adapter_desc
	local err
	local ret=0

	for dr_adapter_desc in "${dp_in_adapters[@]}"; do
		local -A dr_adapter=()
		local dpme
		local dra

		dradapter_deserialize dr_adapter "$dr_adapter_desc"

		dra=$(get_adapter_dra dr_adapter)

		dpme=$(get_dpme dr_adapter)
		err=$?

		[ $ret -eq 0 ] && ret=$err
		if [ $err -ne 0 ]; then
			log_err "Cannot get DPME for DP IN adapter at DRA $dra\n"

			[ $err -eq $ERR_INT ] && break

			dpme_current_cycle_errors=$((dpme_current_cycle_errors + 1))
			dpme_total_errors=$((dpme_total_errors + 1))

			continue
		fi

		if [ "$dpme" != "0x1" ]; then
			log_err "DPME for DP IN adapter at DRA $dra is not enabled\n"
			[ $ret -eq 0 ] && ret=1

			dpme_current_cycle_errors=$((dpme_current_cycle_errors + 1))
			dpme_total_errors=$((dpme_total_errors + 1))
		fi
	done

	return $ret
}

calc_dropped_errors()
{
	local dropped_recs=$1

	es_dropped[ES_CURRENT]=$((dropped_recs - \
				  current_cycle_start[CS_REC]))
	[ ${es_dropped[ES_CURRENT]} -lt 0 ] && \
		es_dropped[ES_CURRENT]=0

	es_dropped[ES_TOTAL]=$dropped_recs
}

calc_error_stats()
{
	local -n __es_calc=$1
	local new_errors=$2

	__es_calc[ES_CURRENT]=$((__es_calc[ES_CURRENT] + new_errors))
	__es_calc[ES_LAST]=${__es_calc[ES_CURRENT]}
	__es_calc[ES_TOTAL]=$((__es_calc[ES_TOTAL] + new_errors))
}

check_dmesg_errors()
{
	local rec_info
	local -a ri
	local rec_counts
	local -a rc
	local dump_recs
	local last_rec
	local err

	rec_info=$(trace_get_record_info) || return $?

	read -a ri < <(echo "$rec_info") || cmd_err_ret

	last_rec=$((ri[RI_FIRST_REC] + ri[RI_NUM_RECS]))
	current_cycle_read[CS_SEQ]=${ri[RI_KMSG_SEQ]}
	current_cycle_read[CS_TIMESTAMP]=${ri[RI_KMSG_TIMESTAMP]}

	dump_recs=$((last_rec - current_cycle_checked[CS_REC]))
	dump_recs=$(min $dump_recs $MAX_REC_SCAN)
	last_rec=$((current_cycle_checked[CS_REC] + dump_recs))

	calc_dropped_errors "${ri[RI_FIRST_REC]}"

	if [ $dump_recs -eq 0 ]; then
		if [ $last_rec -ne 0 ]; then
			current_cycle_checked[CS_SEQ]=${current_cycle_read[CS_SEQ]}
			current_cycle_checked[CS_TIMESTAMP]=${current_cycle_read[CS_TIMESTAMP]}
		fi

		return 0
	fi

	err=0
	rec_counts=$(trace_dump_recs "$TRACE_OP_COUNT" \
				     "$TRACE_FILTER_ONLY" \
				     "$trace_filter_msg_regex" \
				     "$trace_filter_dev_regex" \
				     "${current_cycle_checked[CS_REC]}" "$dump_recs" 0) || err=$?
	[ $err -ne 0 ] && return $err

	read -a rc < <(echo "$rec_counts") || return $?

	last_rec=$((rc[RC_FIRST_REC] + rc[RC_NUM_RECS]))

	calc_dropped_errors ${rc[RC_DROPPED_RECS]}
	calc_error_stats es_dmesg $((rc[RC_NUM_RECS] - rc[RC_MATCH_RECS]))
	calc_error_stats es_dmesg_flt ${rc[RC_MATCH_RECS]}

	current_cycle_checked[CS_REC]=$last_rec
	last_cycle_with_errors=$test_cycle

	if [ ${rc[RC_NUM_RECS]} -gt 0 ]; then
		current_cycle_checked[CS_SEQ]=${rc[RC_LAST_SEQ]}
		current_cycle_checked[CS_TIMESTAMP]=${rc[RC_LAST_TIMESTAMP]}
	fi

	if [ ${current_cycle_start[CS_SEQ]} -eq -1 ]; then
		current_cycle_start[CS_SEQ]=$((${rc[RC_LAST_SEQ]} - rc[RC_NUM_RECS]))
	fi
	last_cycle_start[CS_REC]=${current_cycle_start[CS_REC]}

	return 0
}

check_and_reset_new_errors()
{
	local -n __es_chk=$1
	local old_reported=${__es_chk[ES_REPORTED]}

	__es_chk[ES_REPORTED]=${__es_chk[ES_CURRENT]}

	if [ $old_reported -eq ${__es_chk[ES_REPORTED]} ]; then
		return 0
	fi

	return $ERR_STATS_CHANGED
}

check_all_errors()
{
	check_dmesg_errors || return $?

	check_and_reset_new_errors es_dmesg || return $?
	check_and_reset_new_errors es_dmesg_flt || return $?
	check_and_reset_new_errors es_dpme || return $?
	check_and_reset_new_errors es_other || return $?

	return 0
}

rec_tracker_is_complete()
{
	local idx=$1
	local now=$2

	if [ ${rec_tracker_timestamp[idx]} -eq -1 ]; then
		return $ERR_STATS_CHANGED
	fi

	if [ ${rec_tracker_timestamp[idx]} -ge \
	     ${rec_tracker_target_timestamp[idx]} ]; then \
		return 0
	fi

	if [ $((now - rec_tracker_update_time[idx])) -ge \
	     $REC_TRACKER_POLL_DURATION ]; then
		return 0
	fi

	return $ERR_STATS_CHANGED
}

rec_tracker_update()
{
	local idx=$1
	local now=$2
	local new_seq=$3
	local new_timestamp=$4
	local ret=0

	rec_tracker_is_complete $idx $now && return 0

	if [ ${rec_tracker_target_timestamp[idx]} -eq -1 ]; then
		if [ $idx -eq 0 ]; then
			pr_err "Unexpected rec tracker state\n"
			exit 1
		fi
		rec_tracker_target_timestamp[idx]=$(min \
						    ${rec_tracker_target_timestamp[idx - 1]} \
						    ${rec_tracker_timestamp[idx - 1]})
	fi

	if [ ${rec_tracker_seq[idx]} -eq -1 -o \
	     ${rec_tracker_seq[idx]} -lt $new_seq ]; then
		rec_tracker_seq[idx]=$new_seq
		rec_tracker_timestamp[idx]=$new_timestamp
		rec_tracker_update_time[idx]=$now
	fi

	return $ERR_STATS_CHANGED
}

check_kmsg_dumper()
{
	local now=$1
	local dstats
	local -a ds

	dstats=$(trace_get_kmsg_dumper_info) || return $?

	read -a ds < <(echo "$dstats") || cmd_err_ret

	# Did the given stage start already?
	[ ${ds[DS_SEEK_SEQ]} -lt 0 -o \
	  ${ds[DS_FEED_SEQ]} -lt 0 ] && return $ERR_GENERIC

	rec_tracker_update $RT_SEEK $now \
			   ${ds[DS_SEEK_SEQ]} \
			   ${ds[DS_SEEK_TIMESTAMP]} || return $?

	rec_tracker_update $RT_FEED $now \
			   ${ds[DS_FEED_SEQ]} \
			   ${ds[DS_FEED_TIMESTAMP]} || return $?
}

check_kmsg_parser()
{
	local now=$1

	rec_tracker_update $RT_PARSE $now \
			   ${current_cycle_checked[CS_SEQ]} \
			   ${current_cycle_checked[CS_TIMESTAMP]}
}

# Check if dmesg scanning has settled
check_dmesg_scan()
{
	local now=$(get_epoch_seconds)

	[ $test_phase -ne $TEST_PHASE_PREPARE ] && return 0

	if check_kmsg_dumper "$now" &&
	   check_kmsg_parser "$now"; then
		update_test_phase $TEST_PHASE_RESUMED || return $?

		return 0
	fi

	return 0
}

get_rec_tracker_progress()
{
	local now=$(get_epoch_seconds)
	local delta range perc
	local range_end

	if ! rec_tracker_is_complete $RT_SEEK $now ||
	   [ ${current_cycle_start[CS_SEQ]} -eq -1 ]; then
		echo "$(rpad_space "$(repeat_char "." $((now % 5)))" 4)"
		return
	fi

	delta=$((current_cycle_checked[CS_SEQ] - \
		 current_cycle_start[CS_SEQ]))
	range_end=$(max ${rec_tracker_seq[RT_SEEK]} \
			${rec_tracker_seq[RT_FEED]})
	range=$((range_end - current_cycle_start[CS_SEQ]))

	if [ $range -eq 0 ]; then
		perc=0
	else
		perc=$((delta * 10000 / range))
	fi

	perc=$(($(min $perc $perc) / 100))

	echo "$(lpad_space "$perc" 3) %"
}

sum_errors()
{
	local err_type=$1
	local flags=$2
	local errors=0

	errors=$((errors + es_dmesg[err_type]))
	errors=$((errors + es_dpme[err_type]))
	errors=$((errors + es_other[err_type]))

	[ $((flags & NO_DMESG_FLT)) -eq 0 ] && \
		errors=$((errors + es_dmesg_flt[err_type]))
	[ $((flags & NO_DROPPED)) -eq 0 ] && \
		errors=$((errors + es_dropped[err_type]))

	echo "$errors"
}

phase_flags()
{
	echo "${test_phase_flags[$test_phase]-0}"
}

phase_has_flag()
{
	local flag=$1

	[ $(($(phase_flags) & $flag)) -ne 0 ]
}

update_test_phase_str()
{
	local status=$1
	local commands=$2

	update_back_if_changed "$test_phase_commands_str" "$commands"
	update_back_if_changed "$test_phase_status_str" "$status"

	test_phase_status_str="$status"
	test_phase_commands_str="$commands"

	log_cr
	log "${status}${commands}"
}

erase_test_phase_commands()
{
	cursor_erase_str "$test_phase_commands_str"
	test_phase_commands_str=""
}

erase_test_phase()
{
	erase_test_phase_commands
	cursor_erase_str "$test_phase_status_str"
	test_phase_status_str=""
}

get_prefix()
{
	local label=$1

	echo "${COLOR_GREY}$label +$(sec_to_time $((test_phase_start - test_started)))${COLOR_NONE}"
}

get_cycle_prefix()
{
	get_prefix "Cycle#$test_cycle"
}

get_error_str()
{
	local new_errors=$1
	local total_errors=$2
	local msg=""

	[ $new_errors -ne 0 ] && msg+="$COLOR_RED"
	msg+="$total_errors $(plural_str $total_errors error)"
	[ $new_errors -ne 0 ] && msg+="$COLOR_NONE"

	echo "$msg"
}

get_test_phase_name()
{
	if ! test_state_is_run; then
		echo "${COLOR_YELLOW}Stopped${COLOR_NONE}"
	else
		echo "${test_phase_label[$test_phase]}"
	fi
}

get_commands_str()
{
	local start_stop

	if test_state_is_run; then
		start_stop="$(emph_paren_first "stop")/"
	else
		start_stop="$(emph_paren_first "Start")/"
	fi

	echo "press ${start_stop}$(emph_paren_first "help")"
}

get_progress_indicator()
{
	if phase_has_flag "$HAS_COUNTDOWN"; then
		get_cycle_countdown
	elif phase_has_flag "$HAS_PROGRESS"; then
		get_rec_tracker_progress
	fi
}

print_test_phase()
{
	local new_errors=0
	local total_errors=0
	local status
	local commands=""
	local err=0

	new_errors=$(sum_errors $ES_CURRENT $((NO_DMESG_FLT | NO_DROPPED)))
	total_errors=$(sum_errors $ES_TOTAL $((NO_DMESG_FLT | NO_DROPPED)))

	status="$(get_cycle_prefix): "

	status+=$(get_test_phase_name)
	status+=" $(get_progress_indicator)"
	status+=", $(get_error_str $new_errors $total_errors)"

	phase_has_flag "$HAS_COMMANDS" && \
		commands=", $(get_commands_str)"

	update_test_phase_str "$status" "$commands"
}

get_cycle_countdown()
{
	echo "-$("$PRINTF" "%02ds" $cycle_delay_left)"
}

__update_test_delay()
{
	local now=$1

	if ! test_state_is_run || \
	  [ $test_phase -eq $TEST_PHASE_PREPARE ]; then
		cycle_delay_expires=$((now + cycle_delay_left))
	fi

	cycle_delay_left=$((cycle_delay_expires - now))
	[ $cycle_delay_left -lt 0 ] && cycle_delay_left=0

	print_test_phase
}

update_test_delay()
{
	__update_test_delay $(get_epoch_seconds)
}

reset_test_delay()
{
	cycle_delay_expires=$(get_epoch_seconds)

	__update_test_delay $cycle_delay_expires
}

update_test_phase()
{
	local phase=$1

	test_phase=$phase

	test_phase_start=$(get_epoch_seconds)
	cycle_delay_expires=$((test_phase_start + $TEST_CYCLE_DELAY_SEC))

	__update_test_delay $test_phase_start
}

suspend_and_autoresume()
{
	local ret=0

	if $dry_run; then
		sleep $AUTORESUME_DELAY_SEC
		return 0
	fi

	if $reload_network; then
		test_cmd_no_out $SUDO $RMMOD \
				"$NETWORK_MODULE" || ret=$?
	fi

	if [ $ret -eq 0 ]; then
		test_cmd_no_out $SUDO $RTCWAKE \
				-m mem \
				-s $AUTORESUME_DELAY_SEC || ret=$?
	fi

	if $reload_network; then
		# retry modprobe a few times if it gets interrupted
		test_cmd_retry_no_out $SUDO $MODPROBE \
				      "$NETWORK_MODULE" || ret=$?
	fi

	return $ret
}

poke_remote_with_ping()
{
	local err=0

	"$ping_tool" $ping_options &> /dev/null || err=$?

	[ $err -eq 0 ] && return 0

	if [ $err -eq 2 ]; then
		sleep 1
	fi

	return 1
}

poke_remote_with_nmap()
{
	local output
	local err

	err=0
	output=$($ping_tool $ping_options 2>&1) || err=$?
	if [[ $err -eq 0 ]] && \
	   [[ "$output" =~ $NMAP_STATUS_UP_REGEX ]]; then
		   return 0
	fi

	sleep 1

	return 1
}

poke_remote()
{
	local err
	local ping_output

	if [ "$(basename "$ping_tool")" = "ping" ]; then
		err=0
		poke_remote_with_ping || err=$?
	else
		err=0
		poke_remote_with_nmap || err=$?
	fi

	return $err
}

wait_network_connection()
{
	local wait_expires=$(($(get_epoch_seconds) + \
			      MAX_PING_DURATION_SEC))

	[ -z "$ping_tool" ] && return 0

	while test_state_is_run && \
	      [ $(date +%s) -lt $wait_expires ]; do
		poke_remote && break
	done

	return 0
}

add_new_other_errors()
{
	local err=$1

	[ $err -eq 0 -o $err -eq $ERR_INT ] && return

	es_other[ES_CURRENT]=$((es_other[ES_CURRENT] + 1))
	es_other[ES_TOTAL]=$((es_other[ES_TOTAL] + 1))
}

test_suspend_resume()
{
	local err

	update_test_phase $TEST_PHASE_SUSPENDING || return $?

	err=0
	suspend_and_autoresume || err=$?
	add_new_other_errors "$err"

	err=0
	wait_network_connection || return $?
	add_new_other_errors "$err"

	return 0
}

cleanup_suspend_resume()
{
	erase_test_phase
	log_cr
}

pause_test_phase()
{
	local pause=$1
	local next_state

	if $pause; then
		next_state=$TEST_STATE_PAUSE
	else
		next_state=$TEST_STATE_RUN
	fi

	test_state_is $next_state && return

	update_test_delay
	test_state_set $next_state
	update_test_delay
}

get_error_header()
{
	printf "${error_line_fmt}" "" New Total
}

get_error_line()
{
	local -nr __es_prn=$1
	local label=$2

	printf "${error_line_fmt}\n" \
	       "$label" \
	       "${__es_prn[ES_CURRENT]}" \
	       "${__es_prn[ES_TOTAL]}"
}

print_error_stats()
{
	erase_test_phase

	log_cr
	log "Errors (for all sources, w/o the device filter applied):\n"

	log_cont "$(get_error_header)\n"

	log_cont "$(get_error_line es_dmesg	"Unfiltered Dmesg")\n"
	log_cont "$(get_error_line es_dmesg_flt	"Filtered Dmesg")\n"
	log_cont "$(get_error_line es_dropped	"Dropped Dmesg")\n"
	log_cont "$(get_error_line es_dpme	"DP BWA mode state (DPME)")\n"
	log_cont "$(get_error_line es_other	"Others")\n"
}

declare -Ar msg_filter_label=(
	[$TRACE_FILTER_NONE]="all"
	[$TRACE_FILTER_EXCLUDE]="unfiltered"
	[$TRACE_FILTER_ONLY]="filtered"
)

get_filter_msg_config_str()
{
	echo "${msg_filter_label[$trace_filter_msg_config]}"
}

change_msg_filter_config()
{
	erase_test_phase
	log_cr

	trace_filter_msg_config=$(((trace_filter_msg_config + 1) % TRACE_FILTER_MAX))
	log "$(get_cycle_prefix): Dmesg message filter changed to: $(get_filter_msg_config_str) messages\n"
}

get_filter_dev_str()
{
	case "$trace_filter_dev_mask" in
	$FILTER_DEV_NONE)
		echo "any sources"
		;;
	$FILTER_DEV_TB)
		echo "TBT device"
		;;
	$FILTER_DEV_GFX)
		echo "GFX device"
		;;
	$FILTER_DEV_ALL)
		echo "TBT and GFX devices"
		;;
	*)
		echo "Invalid"
		;;
	esac
}

change_dev_filter()
{
	erase_test_phase
	log_cr

	trace_filter_dev_mask=$(((trace_filter_dev_mask + 1) % \
			        (FILTER_DEV_ALL + 1)))

	log "$(get_cycle_prefix): Dmesg device filter changed to: $(get_filter_dev_str)\n"

	case "$trace_filter_dev_mask" in
	$FILTER_DEV_NONE)
		trace_filter_dev_regex=""
		;;
	$FILTER_DEV_TB)
		trace_filter_dev_regex="$tb_dev_regex"
		;;
	$FILTER_DEV_GFX)
		trace_filter_dev_regex="$gfx_dev_regex"
		;;
	$FILTER_DEV_ALL)
		trace_filter_dev_regex="$tb_dev_regex"
		if [ -n "$gfx_dev_regex" ]; then
			trace_filter_dev_regex+="${trace_filter_dev_regex:+|}$gfx_dev_regex"
		fi
		;;
	*)
		trace_filter_dev_regex=""
	esac
}

dump_recs()
{
	local trace_op=$1
	local since=$2
	local label=${3-}
	local -r indent=2
	local dump_count
	local dump_start=$since

	erase_test_phase
	log_cr

	[ $dump_start -lt 0 ] && dump_start=0

	if trace_is_clear_only_op "$trace_op"; then
		log "$(get_cycle_prefix): Clearing all dmesg messages\n"
	else
		log "$(get_cycle_prefix): Dmesg$label:\n"
	fi

	trace_dump_recs "$trace_op" \
			"$trace_filter_msg_config" \
			"$trace_filter_msg_regex" \
			"$trace_filter_dev_regex" \
			"$since" "-1" "$indent" || true
}

dump_last_cycle_recs()
{
	local label

	label="Cycle#$last_cycle_with_errors, $(get_filter_msg_config_str), $(get_filter_dev_str))"

	if [ ${es_dmesg[ES_LAST]} -eq 0 -a \
	     ${es_dmesg_flt[ES_LAST]} -eq 0 ]; then
		log "$(get_cycle_prefix): No dmesg messsages to show for $label\n"
		return
	fi

	dump_recs "$TRACE_OP_PRINT" "${last_cycle_start[CS_REC]}" " for $label"
}

dump_all_recs()
{
	local label

	label=" (since the test started, $(get_filter_msg_config_str), $(get_filter_dev_str))"

	if [ ${es_dmesg[ES_TOTAL]} -eq 0 -a \
	     ${es_dmesg_flt[ES_TOTAL]} -eq 0 ]; then
		log "$(get_cycle_prefix): No dmesg messsages to show\n"
		return
	fi

	dump_recs "$TRACE_OP_PRINT" "0" "$label"
}

print_help()
{
	erase_test_phase
	log_cr
	declare -a commands=(
		"d:Dump last dmesg errors"
		"D:Dump all dmesg errors"
		"f:Change dmesg message filtering (current: $(get_filter_msg_config_str) messages)"
		"F:Change dmesg device filtering (current: $(get_filter_dev_str))"
		"c:Print and clear all dmesg errors"
		"C:Clear all dmesg errors"
		"e:Print error stats"
		"s:Stop test cycle"
		"S:Start test cycle (pressed again: skip delay)"
		"q:Quit test"
		"h:Print this help"
	)

	log "Usage:\n"
	for cmd in "${commands[@]}"; do
		log "  $(emph_first "${cmd%%:*}") - ${cmd#*:}\n"
	done
}

print_invalid_command()
{
	local sym=$(get_symbol_for_char "$1")
	local msg=" ${COLOR_RED}$sym ?${COLOR_NONE}"
	local i

	for ((i = 0; i < 1; i++)) {
		echo -ne "$msg"
		sleep 0.1

		cursor_erase_str "$msg"
		[ $i -lt 2 ] && sleep 0.05
	}
}

handle_command()
{
	local char=$1
	local pause=true

	case "$char" in
	d)
		dump_last_cycle_recs
		;;
	D)
		dump_all_recs
		;;
	f)
		change_msg_filter_config
		;;
	F)
		change_dev_filter
		;;
	c)
		dump_recs "$TRACE_OP_PRINT_AND_CLEAR" 0
		;;
	C)
		dump_recs "$TRACE_OP_CLEAR" 0
		;;
	e)
		print_error_stats
		;;
	s)
		;;
	S)
		if test_state_is_run; then
			reset_test_delay
			pause=
		else
			pause=false
		fi
		;;
	q)
		test_state_set $TEST_STATE_QUIT
		pause=
		;;
	h)
		print_help
		;;
	*)
		print_invalid_command "$char"
		;;
	esac

	if [ -n "$pause" ]; then
		pause_test_phase "$pause"
	fi
}

wait_command()
{
	local err=0

	char=$(read_char 0.3)
	if [ -n "$char" ]; then
		flush_input
		handle_command "$char"
	fi

	check_dmesg_scan

	check_all_errors || err=$?

	[ $err -ne 0 -a $err -ne $ERR_STATS_CHANGED ] && return $err

	update_test_delay

	case $test_state in
	$TEST_STATE_BREAK | $TEST_STATE_QUIT)
		return $ERR_GENERIC
		;;
	$TEST_STATE_PAUSE)
		return 0
		;;
	$TEST_STATE_RUN)
		if [ $test_phase -eq $TEST_PHASE_PREPARE ]; then
			return 0
		elif $skip_test; then
			log "Init complete, skip test as requested\n"
			test_state_set $TEST_STATE_QUIT

			return $ERR_GENERIC
		elif [ $cycle_delay_left -le 0 ]; then
			return $ERR_GENERIC
		else
			return 0
		fi
		;;
	esac
}

run_test_cycle()
{
	local ret=0

	if [ $max_cycle_count -ne 0 -a \
	     $test_cycle -ge $max_cycle_count ]; then
		return $ERR_GENERIC
	fi

	update_test_phase $TEST_PHASE_PREPARE || return $?
	check_dpme || true

	while wait_command; do
		:
	done;

	case $test_state in
	$TEST_STATE_RUN)
		test_cycle=$((test_cycle + 1))

		test_suspend_resume
		;;
	$TEST_STATE_BREAK)
		test_state_set $TEST_STATE_PAUSE
		;;
	$TEST_STATE_QUIT)
		ret=$ERR_GENERIC
		;;
	esac

	return $ret
}

test_cycle_init()
{
	local i

	es_dmesg[ES_CURRENT]=0
	es_dmesg_flt[ES_CURRENT]=0
	es_dpme[ES_CURRENT]=0
	es_other[ES_CURRENT]=0

	if [ ${current_cycle_start[CS_SEQ]} -ne -1 ]; then
		current_cycle_start[CS_SEQ]=${current_cycle_checked[CS_SEQ]}
	fi
	current_cycle_start[CS_REC]=${current_cycle_checked[CS_REC]}

	for i in $RT_SEEK $RT_FEED $RT_PARSE; do
		rec_tracker_seq[i]=-1
		rec_tracker_timesamp[i]=-1
		rec_tracker_target_timesamp[i]=-1
	done

	# consecutive trackers will set their target timestamp to the
	# previous tracker's target timestamp once that tracker is
	# complete.
	rec_tracker_target_timestamp[RT_SEEK]=$((($(get_uptime_sec) + \
						  REC_TRACKER_POLL_DURATION) * \
						 USEC_PER_SEC))
}

test_cycle_cleanup()
{
	local current_errors=$(sum_errors $ES_CURRENT $((NO_DMESG_FLT | NO_DROPPED)))

	# Leave a trace of separate log line for cycles with errors
	if [ $current_errors -ne 0 ]; then
		erase_test_phase_commands
		log
	fi
}

run_test()
{
	local total_errors
	local msg=""
	local err

	log "$(get_prefix Init): Test started at $(sec_to_date_time "$(get_epoch_seconds)")\n"

	err=0
	while [ $err -eq 0 ]; do
		test_cycle_init
		run_test_cycle || err=$?
		test_cycle_cleanup
	done

	cleanup_suspend_resume

	total_errors=$(sum_errors $ES_TOTAL $((NO_DMESG_FLT | NO_DROPPED)))

	msg+="$test_cycle $(plural_str $test_cycle cycle), "
	msg+="$(get_error_str $total_errors $total_errors)"
	log "$(get_prefix End): $msg\n"

	[ $total_errors -ne 0 ] && print_error_stats "$test_cycle"
}

load_one_module()
{
	local module=$1
	local ret=0
	local loaded_mode
	local mod_list

	mod_list=$(test_cmd $LSMOD) || return 1

	log "$(get_prefix Init): Loading module $module:"

	loaded_mod=$(test_cmd grep_no_err "\<$module\>" <<< "$mod_list") || return 1

	if [ -n "$loaded_mod" ]; then
		log_cont " Already loaded\n"

		return 2
	fi

	test_cmd_no_out $SUDO $MODPROBE "$module" || return 1
	log_cont " Loaded succesfully\n"

	return 0
}

load_modules()
{
	local had_to_modprobe=false
	local module

	for module in $modules; do
		local err=0

		load_one_module "$module" || err=$?
		if [ $err -eq 0 ]; then
			had_to_modprobe=true
		elif [ $err -ne 2 ]; then
			log_err "Loading modules failed: $err\n"

			return $err
		fi
	done

	if $had_to_modprobe; then
		sleep 3
	fi

	return 0
}

init_test()
{
	init_test_start

	init_filter_regex

	log "$(get_prefix "Init"): Reading in dmesg:"
	if ! trace_init "$FILTERED_DMESG_ERRORS"; then
		pr_err "Initialization of dmesg trace failed\n"
		return 1
	fi
	log_cont " done.\n"

	if ! load_modules; then
		trace_cleanup
		return 1
	fi

	if ! init_dp_in_adapters; then
		trace_cleanup
		return 1
	fi

	init_dev_regex

	return 0
}

cleanup_test()
{
	trace_cleanup || true
}

process_is_descendant()
{
	local target_pid=$1
	local current_pid=$BASHPID
	local -a pids=()
	local pid

	[ $target_pid -eq $current_pid ] && return 1

	while [ "$target_pid" -gt 1 ]; do
		if [ "$target_pid" -eq "$current_pid" ]; then
			return 0
		fi

		if [ -n "${non_descendant_pids[$target_pid]+x}" ]; then
			return 1
		fi

		pids+=($target_pid)

		target_pid=$(ps -o ppid= -p $target_pid)

		[ -z "$target_pid" ] && break
	done

	for pid in ${pids[@]}; do
		non_descendant_pids[$pid]=1
	done

	return 1
}

check_descendant_processes()
{
	local pid
	local -a descendants=()

	if ! debug_is_enabled; then
		return
	fi

	for pid in $(ps -axo pid | tail -n +2); do
		if process_is_descendant "$pid"; then
			descendants+=($pid)
		fi
	done

	if [ ${#descendants[@]} -ne 0 ]; then
		local pattern=""

		pr_err "Still active descendant processes:\n"

		for pid in "${descendants[@]}"; do
			# filter out the grep process
			pattern="${pattern}${pattern:+\\|}[${pid:0:1}]${pid:1}"
		done

		ps --forest -ax | grep "$pattern"
	fi
}

main_cleanup()
{
	cleanup_test || true
	cleanup_utils

	check_descendant_processes
}

setup_sig_handlers()
{
	local sigs=$@

	for sig in $sigs; do
		trap "sig_handler $sig" "$sig"
	done
}

restore_sig_handlers()
{
	local sigs=$@

	trap - $sigs
}

handle_graceful_sig()
{
	local sig=$1

	case "$successive_sig_count" in
	0)
		log "${COLOR_YELLOW}Test ${sig_causes[$sig]}${COLOR_NONE}\n"
		if test_state_must_break; then
			test_state_set $TEST_STATE_QUIT
		else
			test_state_set $TEST_STATE_BREAK
		fi
		;;
	1)
		pr_err "Next signal will abort\n"
		restore_sig_handlers $graceful_sigs
		;;
	*)
		pr_err "Unexpected signal\n"
		;;
	esac

	if test_state_must_quit; then
		successive_sig_count=$((successive_sig_count + 1))
	fi

	return 0
}

handle_abort_sig()
{
	local sig=$1

	test_state_set $TEST_STATE_QUIT

	pr_err "\nAbort due to script error, forced abort or interrupted initialization\n"

	restore_sig_handlers $all_sigs

	main_cleanup || pr_err "Cleanup failed\n"

	exit 1
}

sig_is_of_type()
{
	local queried_sig=$1
	local sig_list=$2
	local sig

	for sig in $sig_list; do
		[ "$sig" = "$queried_sig" ] && return 0
	done

	return 1
}

sig_handler()
{
	local sig=$1
	local graceful_sig

	debug "Main thread sig \"$sig\"\n"

	if sig_is_of_type "$sig" "$abort_sigs"; then
		handle_abort_sig "$sig"

		return 0
	fi

	handle_graceful_sig "$sig"

	# Trap for the unwary, in the case where set -e
	# is in effect (although even set +e may cause
	# surprise): # an implicit return here
	# would return the value of $? as it was right
	# before the handler got called:
	# https://github.com/jsoref/bash/blob/bc007799f0e13/CWRU/changelog#L5955
	# Which would probably cause an unintended
	# immediate exit of the script even though the
	# interrupted command was part of a conditional,
	# since the trap handler inherits the non-zero
	# (EINTR or the like) $? error code of the
	# interrupted command.
	#
	# The above also applies to all functions called
	# from this handler, so an implicit return from
	# those will cause an immediate exit (unless the
	# call is part of a conditional), even though in
	# normal, non-signal handler context this return
	# value would be 0.

	return 0
}

main_init()
{
	cache_sudo_right || return 1

	init_utils || return 1
	init_test_phase_labels

	setup_sig_handlers $graceful_sigs

	if ! init_test; then
		restore_sig_handlers $graceful_sigs

		cleanup_utils

		return 1
	fi

	return 0
}

init_suspend_resume_tools || err_exit "Init failed\n"
parse_options "$@" || exit $?

setup_sig_handlers $abort_sigs

if ! main_init "$@"; then
	restore_sig_handlers $abort_sigs
	err_exit "Initialization failed\n"
fi

run_test || true   # clean up still

restore_sig_handlers $all_sigs

main_cleanup || err_exit "Cleanup failed\n"

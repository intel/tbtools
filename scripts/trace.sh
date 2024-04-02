KMSG_FILE="/dev/kmsg"

TRACE_TMP_DIR=

TRACE_LOCK_FILE=
TRACE_INFO_FILE=
TRACE_REC_FILE=

KMSG_COUNTER_PID_FILE=
KMSG_DUMPER_PID_FILE=
KMSG_INFO_FILE=
KMSG_LINE_COUNT_FILE=

KMSG_DUMPER_LOCK_FILE=
KMSG_DUMPER_SEEK_FILE=
KMSG_DUMPER_FEED_FILE=
KMSG_DUMPER_EXIT_FILE=

# last number of kmsg lines checked at startup
KMSG_STARTUP_LAST_LINES=10

KMSG_DUMPER_INFO_UPDATE_PERIOD=1000

# 100ms maximum backward step in non-consecutive kmsg
# timestamps
KMSG_MAX_TIMESTAMP_DELTA_US=$((100 * 1000))

declare -r TRACE_MAX_LINES=10000
declare -r TRACE_SLACK=100
declare -r MAX_BATCH_RECS=200
# must cover any line /dev/kmsg wants to return in one read
declare -r KMSG_MIN_BLOCK_SIZE=2048

declare -r TRACE_OP_PRINT=0
declare -r TRACE_OP_COUNT=1
declare -r TRACE_OP_PRINT_AND_CLEAR=2
declare -r TRACE_OP_CLEAR=3

declare -r TRACE_FILTER_NONE=0
declare -r TRACE_FILTER_EXCLUDE=1
declare -r TRACE_FILTER_ONLY=2
declare -r TRACE_FILTER_MAX=3

declare -r REC_FILTER_MATCH=0
declare -r REC_FILTER_UNMATCH=1

declare -r TRACE_FLAG_CLEARED=1

declare -r ERR_GENERIC=240
declare -r ERR_REC_FILE_CHANGED=241
declare -r ERR_READ_TIMEOUT=242

# record info
declare -r RI_FIRST_REC=0
declare -r RI_NUM_RECS=1
declare -r RI_KMSG_SEQ=2
declare -r RI_KMSG_TIMESTAMP=3
declare -r RI_FLAGS=4

# dmesg dump info
declare -r DI_REC=0
declare -r DI_SEQ=1
declare -r DI_TIMESTAMP=2
declare -r DI_SIZE=3

# dmesg dump stats
declare -r DS_SEEK_REC=0
declare -r DS_SEEK_SEQ=1
declare -r DS_SEEK_TIMESTAMP=2
declare -r DS_FEED_REC=3
declare -r DS_FEED_SEQ=4
declare -r DS_FEED_TIMESTAMP=5

# dmesg record
declare -r DR_SUBSYSTEM=0
declare -r DR_DEVICE=1
declare -r DR_PRIO=2
declare -r DR_SEQ=3
declare -r DR_TIMESTAMP=4
declare -r DR_CALLER_ID=5
declare -r DR_MESSAGE=6

# record counter
declare -r RC_FIRST_REC=0
declare -r RC_NUM_RECS=1
declare -r RC_DROPPED_RECS=2
declare -r RC_MATCH_RECS=3
declare -r RC_LAST_SEQ=4
declare -r RC_LAST_TIMESTAMP=5

declare -r max_prio=4

# kmsg batch read info
declare -r KB_RECS_READ=0
declare -r KB_RECS_ADDED=1
declare -r KB_SEQ=2
declare -r KB_TIMESTAMP=3
declare -r KB_LAST_LINE=4

declare -a kmsg_batch=(0 0 -1 -1 "")

trace_server_kmsg_dumper_pids=
trace_server_pid=""

declare -a trace_files=()

declare -r kmsg_counter_poll_period_ms=300
kmsg_counter_last_updated=

valid_number()
{
	[ -n "$1" -a -z "${1/#+([0-9])/}" ]
}

if ! shopt -p expand_aliases > /dev/null; then
	echo "Missing required 'shopt -s expand_aliases'" >&2
	exit 1
fi

alias cmd_err_ret='return $(to_err $?)'
alias get_err='{ err=$?; [ $err -eq 0 ]; }'

kmsg_dumper_exiting()
{
	[ -n "${KMSG_DUMPER_EXIT_FILE:+x}" -a \
	  -f "${KMSG_DUMPER_EXIT_FILE}" ]
}

trace_get_kmsg_dumper_info_locked()
{
	local -a di_seek
	local -a di_feed

	read -a di_seek < "$KMSG_DUMPER_SEEK_FILE"
	read -a di_feed < "$KMSG_DUMPER_FEED_FILE"

	echo "${di_seek[@]} ${di_feed[@]}"
}

trace_get_kmsg_dumper_info()
{
	call_with_flock "$KMSG_DUMPER_LOCK_FILE" \
			trace_get_kmsg_dumper_info_locked
}

trace_write_dumper_info_locked()
{
	local info_file=$1
	local -nr __di_wr_locked=$2

	assert_array_size "dump_info" __di_wr_locked $DI_SIZE
	echo "${__di_wr_locked[@]}" > "$info_file" || cmd_err_ret
}

trace_write_dumper_info()
{
	local info_file=$1
	local -nr __di_wr=$2

	call_with_flock "$TRACE_LOCK_FILE" \
		trace_write_dumper_info_locked "$info_file" __di_wr
}

parse_kmsg_header()
{
	local line=$1
	local prio seq timestamp

	prio=${line%%,*}
	line=${line#*,}

	seq=${line%%,*}
	line=${line#*,}

	timestamp=${line%%,*}

	if ! valid_number "$prio" || \
	   ! valid_number "$seq" || \
	   ! valid_number "$timestamp"; then
		log_kmsg_err "Invalid kmsg header (prio: \"$prio\")" "$seq" "$timestamp"

		return $ERR_GENERIC
	fi

	echo "$prio $seq $timestamp"
}

write_rec_counter_info()
{
	local info_file=$1
	local recno=$2
	local line=$3
	local header
	local -a di

	if [ $((SECONDS - kmsg_counter_last_updated)) -lt 2 ]; then
		return
	fi
	kmsg_counter_last_updated=$SECONDS

	header=$(parse_kmsg_header "$line") || return $?
	read -a di < <(echo "$header")

	trace_write_dumper_info "$info_file" di
}

is_cont_line()
{
	[ "${1:0:1}" = " " ]
}

get_defragged_line()
{
	local tmo_ms=$1
	local line=""

	while true; do
		local frag
		local frag_err
		local err=0

		IFS= read -r -t $(msec_to_sec $tmo_ms) frag || err=$(to_err $?)

		case "$err" in
		0)

			line+="$frag"

			if [ -z "$line" ]; then
				log_err "Read empty dmesg line\n"
				return $ERR_READ_TIMEOUT
			fi

			echo "$line"

			return 0
			;;
		$ERR_ALRM)
			# In reality not a SIG_ALRM, rather pselect
			# failing just no other way to report this event
			# probably
			#
			#
			if [ -n "$frag" ]; then
				line+="$frag"

				# give more time for additional
				# fragments
				tmo=1

				continue
			fi

			err=$ERR_READ_TIMEOUT

			;& # fallthrough
		*)

			if [ -n "$line" ]; then
				log_err "Drop incomplete dmesg line (err:$err):\"$line\"\n"
			fi

			return $err
			;;
		esac
	done
}

count_recs()
{
	local rec_info_file=$1
	local recno=0
	local last_line=""

	while ! kmsg_dumper_exiting; do
		local err=0

		line=$(get_defragged_line $kmsg_counter_poll_period_ms) || err=$?

		case $err in
		0)
			recno=$((recno + 1))

			if ! is_cont_line "$line"; then
				last_line=$line
			fi

			;&  # fallthrough
		$ERR_READ_TIMEOUT)
			if [ -n "$last_line" ]; then
				write_rec_counter_info "$rec_info_file" \
						       $recno \
						       "$last_line"
			fi
			;;
		*)
			if ! kmsg_dumper_exiting; then
				log_err "Read failed with error $err in kmsg record counter, exiting\n"
			fi
			break
		esac
	done
}

dump_kmsg_recs()
{
	local line_offset=$1
	local pid_file=$2

	$SUDO "$STDBUF" -oL \
		"$TAIL" -n +$line_offset \
		"$KMSG_FILE" &
	echo "$!" > "$pid_file"
}

count_kmsg_recs()
{
	local line_offset=$1
	local pid_file=$2
	local rec_info_file=$3

	dump_kmsg_recs $line_offset \
		       "$pid_file" | \
		count_recs "$rec_info_file" || cmd_err_ret
}

get_kmsg_info()
{
	local last_lines=""
	local -a di_seek=()
	local line

	KMSG_LINE_COUNT_FILE="$TRACE_TMP_DIR/line_count"

	err=0
	last_lines=$($SUDO "$DD" if="$KMSG_FILE" \
				 iflag=nonblock \
				 bs=$KMSG_MIN_BLOCK_SIZE \
				 2> /dev/null | \
		      "$TEE" >("$WC" -l > "$KMSG_LINE_COUNT_FILE") | \
		      "$TAIL" -n $KMSG_STARTUP_LAST_LINES
	) || err=$?

	[ -f "$KMSG_LINE_COUNT_FILE" ] || return $ERR_GENERIC

	di_seek[DI_REC]=$("$CAT" "$KMSG_LINE_COUNT_FILE") || cmd_err_ret
	"$RM" "$KMSG_LINE_COUNT_FILE" || cmd_err_ret
	KMSG_LINE_COUNT_FILE=""

	valid_number "${di_seek[DI_REC]}" || return $ERR_GENERIC

	[ -n "$last_lines" ] || return $ERR_GENERIC

	# Get the seq/timestamp info from the last record that has this
	# information
	while IFS= read -r line; do
		local header
		local prio

		is_cont_line "$line" && continue

		header=$(parse_kmsg_header "$line") || return $?
		read prio di_seek[DI_SEQ] di_seek[DI_TIMESTAMP] < <(echo "$header")
	done < <(echo "$last_lines")

	[ ${#di_seek[@]} -eq $DI_SIZE ] || return $ERR_GENERIC

	echo "${di_seek[@]}"
}

trace_kmsg_dumper_sig()
{
	local pid
	local pid_file

	debug "Trace kmsg dumper sig\n"

	> "$KMSG_DUMPER_EXIT_FILE" || true
	sleep $(msec_to_sec $((kmsg_counter_poll_period_ms * 2))) # wait for the counter process to exit
	"$RM" -f "$KMSG_DUMPER_EXIT_FILE" || true

	for pid_file in "$KMSG_DUMPER_PID_FILE" "$KMSG_COUNTER_PID_FILE"; do
		pid=$(cat "$pid_file" 2> /dev/null) || true

		if [ -n "$pid" ]; then
			"$KILL" "$pid" 2> /dev/null || true
			wait "$pid" 2> /dev/null || true
		fi

		"$RM" -f "$pid_file" || true
	done

	if [ -n "${KMSG_INFO_FILE:+x}" ]; then
		"$RM" -f "$KMSG_INFO_FILE" 2> /dev/null
	fi

	if [ -n "${KMSG_LINE_COUNT_FILE:+x}" ]; then
		"$RM" -f "$KMSG_LINE_COUNT_FILE" 2> /dev/null
	fi

	exit 0
}

trace_kmsg_dumper()
{
	local -a di_seek
	local kmsg_info
	local line_offset

	set -eu

	KMSG_COUNTER_PID_FILE="$TRACE_TMP_DIR/counter-pid"
	KMSG_DUMPER_PID_FILE="$TRACE_TMP_DIR/dumper-pid"
	KMSG_DUMPER_EXIT_FILE="$TRACE_TMP_DIR/dumper-exiting"

	trap trace_kmsg_dumper_sig EXIT

	err=0
	KMSG_INFO_FILE="$TRACE_TMP_DIR/kmsg-info"
	get_kmsg_info > "$KMSG_INFO_FILE" || return $?

	read -a di_seek < "$KMSG_INFO_FILE" || return $?

	"$RM" "$KMSG_INFO_FILE" || return $?
	KMSG_INFO_FILE=""

	line_offset=$((di_seek[DI_REC] - KMSG_STARTUP_LAST_LINES))
	[ $line_offset -lt 0 ] && line_offset=0

	trace_write_dumper_info "$KMSG_DUMPER_SEEK_FILE" \
				di_seek

	count_kmsg_recs $line_offset \
			"$KMSG_COUNTER_PID_FILE" \
			"$KMSG_DUMPER_FEED_FILE" &

	dump_kmsg_recs $line_offset \
		       "$KMSG_DUMPER_PID_FILE"

	wait
}

trace_write_info()
{
	local -nr __ri_write=$1

	echo "${__ri_write[@]}" > "$TRACE_INFO_FILE" || cmd_err_ret
}

trace_clear_recs()
{
	local -nr __ri_clear=$1
	local -a ri=("${__ri_clear[@]}")

	ri[RI_FLAGS]=$((ri[RI_FLAGS] | TRACE_FLAG_CLEARED))

	trace_write_info ri
}

trace_update_rec_info()
{
	local -a ri

	read -a ri < "$TRACE_INFO_FILE" || cmd_err_ret

	if [ ${kmsg_batch[KB_RECS_ADDED]} -eq 0 -a \
	     ${kmsg_batch[KB_SEQ]}	 -eq ${ri[RI_KMSG_SEQ]} -a \
	     ${kmsg_batch[KB_TIMESTAMP]} -eq ${ri[RI_KMSG_TIMESTAMP]} ]; then
		return 0
	fi

	ri[RI_NUM_RECS]=$((ri[RI_NUM_RECS] + kmsg_batch[KB_RECS_ADDED]))
	ri[RI_KMSG_SEQ]=${kmsg_batch[KB_SEQ]}
	ri[RI_KMSG_TIMESTAMP]=${kmsg_batch[KB_TIMESTAMP]}

	trace_write_info ri
}

self_fd()
{
	echo "/proc/self/fd/$1"
}

trace_truncate_recs()
{
	local keep_lines
	local cleared=false
	local err
	local tmp_file
	local in_fd_path=$(self_fd "$TRACE_REC_IN_FD")
	local out_fd_path=$(self_fd "$TRACE_REC_OUT_FD")
	local -a ri

	read -a ri < "$TRACE_INFO_FILE" || cmd_err_ret

	[ $((ri[RI_FLAGS] & TRACE_FLAG_CLEARED)) -ne 0 ] && cleared=true

	if ! $cleared && \
	   [ ${ri[RI_NUM_RECS]} -lt $((TRACE_MAX_LINES + TRACE_SLACK)) ]; then
		return 0
	fi

	if $cleared; then
		keep_lines=${kmsg_batch[KB_RECS_ADDED]}
		ri[RI_FLAGS]=$((ri[RI_FLAGS] & ~TRACE_FLAG_CLEARED))
	else
		keep_lines=$TRACE_MAX_LINES
	fi

	# below is only for consistency, not needed for now
	[ $keep_lines -gt ${ri[RI_NUM_RECS]} ] && keep_lines=${ri[RI_NUM_RECS]}

	tmp_file=$(mktemp) || cmd_err_ret

	"$DD" if="$in_fd_path" 2> /dev/null | \
		"$TAIL" -n "$keep_lines" > "$tmp_file" || cmd_err_ret

	"$DD" if="$tmp_file" of="$out_fd_path" 2> /dev/null || cmd_err_ret

	"$RM" -f "$tmp_file" || cmd_err_ret

	ri[RI_FIRST_REC]=$((ri[RI_FIRST_REC] + ri[RI_NUM_RECS] - keep_lines))
	ri[RI_NUM_RECS]=$keep_lines

	trace_write_info ri || return $?

	return $ERR_REC_FILE_CHANGED
}

update_rec_file_locked()
{
	trace_update_rec_info || return $?
	trace_truncate_recs
}

trace_get_record_info_locked()
{
	local -a ri
	local -a ret
	local first_rec num_recs

	read -a ri < "$TRACE_INFO_FILE" || cmd_err_ret

	first_rec=${ri[RI_FIRST_REC]}
	num_recs=${ri[RI_NUM_RECS]}

	if [ $((ri[RI_FLAGS] & TRACE_FLAG_CLEARED)) -ne 0 ]; then
		first_rec=$((first_rec + num_recs))
		num_recs=0
	fi

	ret=($first_rec
	     $num_recs
	     ${ri[RI_KMSG_SEQ]}
	     ${ri[RI_KMSG_TIMESTAMP]})

	echo "${ret[@]}"
}

trace_get_record_info()
{
	call_with_flock "$TRACE_LOCK_FILE" trace_get_record_info_locked
}

trace_server_sig_exit()
{
	local pid

	debug "Trace server sig\n"

	trap - EXIT INT

	for pid in $trace_server_kmsg_dumper_pids; do
		"$KILL" "$pid" 2> /dev/null || true
		wait "$pid" 2> /dev/null || true
	done

	"$RM" "$trace_server_kmsg_dumper_fifo"

	exit 0
}

get_dmesg_device()
{
	local line=$1
	local device

	device=${line# DEVICE=}
	[ "$device" = "$line" ] && device=""

	echo "$device"

	[ -n "$device" ]
}

get_dmesg_subsystem()
{
	local line=$1
	local subsystem

	subsystem=${line# SUBSYSTEM=}

	[ "$subsystem" = "$line" ] && subsystem=""

	echo "$subsystem"

	[ -n "$subsystem" ]
}

check_kmsg_prio()
{
	local prio=$1

	if ! valid_number "$prio"; then
		log_err "Invalid dmesg priority: $prio\n"
		return 1
	fi

	if [ $prio -gt $max_prio ]; then
		return 1
	fi

	return 0
}

log_kmsg_err()
{
	local label=$1
	local seq=$2
	local timestamp=$3

	log_err "$label: \"$seq\"/\"$timestamp\" (batch: ${kmsg_batch[KB_SEQ]}/${kmsg_batch[KB_TIMESTAMP]})\n"
}

check_kmsg_header()
{
	local line=$1
	local prio seq timestamp

	prio=${line%%,*}
	line=${line#*,}

	seq=${line%%,*}
	line=${line#*,}

	timestamp=${line%%,*}

	kmsg_batch[KB_RECS_READ]=$((kmsg_batch[KB_RECS_READ] + 1))

	if [ ${kmsg_batch[KB_SEQ]} -eq -1 ]; then
		kmsg_batch[KB_SEQ]=$((seq - 1))
		kmsg_batch[KB_TIMESTAMP]=$timestamp
	fi

	if [ "$seq" -le ${kmsg_batch[KB_SEQ]} ] ||
	   [ "$seq" -gt $((kmsg_batch[KB_SEQ] + 1000)) ] ||
	   [ "$timestamp" -lt \
	     $((${kmsg_batch[KB_TIMESTAMP]} - KMSG_MAX_TIMESTAMP_DELTA_US)) ]; then
		log_kmsg_err "Non-consecutive kmsg seq/timestamp" "$seq" "$timestamp"

		return $ERR_GENERIC
	fi

	if [ "$seq" -ne $((kmsg_batch[KB_SEQ] + 1)) ]; then
		log_kmsg_err "Resetting kmsg seq" "$seq" "$timestamp"
	fi

	kmsg_batch[KB_SEQ]=$seq
	kmsg_batch[KB_TIMESTAMP]=$timestamp

	check_kmsg_prio "$prio"
}

process_dmesg_line()
{
	local line=$1
	local cont_lines=$2

	[ -n "$line" ] || return 0

	check_kmsg_header "$line" || return 0

	echo "$cont_lines,$line" >& "$TRACE_REC_OUT_FD" || cmd_err_ret

	kmsg_batch[KB_RECS_ADDED]=$((kmsg_batch[KB_RECS_ADDED] + 1))

	return 0
}

kmsg_batch_emit()
{
	local i
	local ret=""

	for ((i = KB_RECS_READ; i < KB_LAST_LINE; i++)); do
		ret+="${ret:+,}${kmsg_batch[i]}"
	done

	ret+=";${kmsg_batch[KB_LAST_LINE]}"

	echo "$ret"
}

kmsg_batch_get()
{
	local info=$1
	local -n __kb_get=$2
	local header=${info%%;*}

	IFS=, read -a __kb_get < <(echo "$header") || cmd_err_ret
	__kb_get[KB_LAST_LINE]=${info#*;}
}

process_dmesg_line_batch()
{
	local line
	local cont_lines=""
	local err=0

	if [ ${kmsg_batch[KB_RECS_READ]} -ne 0 -o \
	     ${kmsg_batch[KB_RECS_ADDED]} -ne 0 ]; then
	     pr_err "Unexpected batch read/added recs: ${kmsg_batch[@]}\n"

	     kmsg_batch[KB_RECS_READ]=0
	     kmsg_batch[KB_RECS_ADDED]=0
	fi

	line=${kmsg_batch[KB_LAST_LINE]}
	kmsg_batch[KB_LAST_LINE]=""

	while true; do
		if [ -z "$line" ] && \
		   ! { line=$(get_defragged_line 100) || get_err }; then
			break
		fi

		if is_cont_line "$line"; then
			cont_lines+="$line"
			line=""

			continue
		fi

		process_dmesg_line "${kmsg_batch[KB_LAST_LINE]}" "$cont_lines" || return $?

		kmsg_batch[KB_LAST_LINE]=$line

		if [ ${kmsg_batch[KB_RECS_READ]} -ge $MAX_BATCH_RECS -o \
		     ${kmsg_batch[KB_RECS_ADDED]} -ge $MAX_BATCH_RECS ]; then
			kmsg_batch_emit

			return 0
		fi

		cont_lines=""
		line=""
	done

	[ $err -ne $ERR_READ_TIMEOUT ] && return $err

	process_dmesg_line "${kmsg_batch[KB_LAST_LINE]}" "$cont_lines" || return $?
	kmsg_batch[KB_LAST_LINE]=""

	kmsg_batch_emit

	return $ERR_READ_TIMEOUT
}

reopen_fds()
{
	exec {TRACE_REC_OUT_FD}>&- || cmd_err_ret
	exec {TRACE_REC_IN_FD}>&- || cmd_err_ret

	exec {TRACE_REC_IN_FD}< "$TRACE_REC_FILE" || cmd_err_ret
	exec {TRACE_REC_OUT_FD}>> "$TRACE_REC_FILE" || cmd_err_ret
}

update_rec_file()
{
	local err=0

	call_with_flock "$TRACE_LOCK_FILE" \
			update_rec_file_locked || err=$?

	kmsg_batch[KB_RECS_ADDED]=0
	kmsg_batch[KB_RECS_READ]=0

	[ $err -ne $ERR_REC_FILE_CHANGED ] && return $err

	reopen_fds
}

trace_server()
{
	local last_line

	shopt -s extglob
	set -eu

	trap "" INT # handled by the main thread
	trap trace_server_sig_exit EXIT

	trace_server_kmsg_dumper_fifo="$TRACE_TMP_DIR/fifo"
	[ -p "$trace_server_kmsg_dumper_fifo" ] || \
		mkfifo "$trace_server_kmsg_dumper_fifo" || cmd_err_ret

	while IFS= read -r kmsg_batch[KB_LAST_LINE]; do
		local kmsg_info
		local recs_added
		local err=0

		while kmsg_info="$(process_dmesg_line_batch)" || get_err; do
			kmsg_batch_get "$kmsg_info" kmsg_batch || return $?

			update_rec_file || return $?
		done

		if [ $err -ne 0 -a $err -ne $ERR_READ_TIMEOUT ]; then
			pr_err "Trace server loop: unexpected error:$err, exiting\n"
			return $err
		fi

		kmsg_batch_get "$kmsg_info" kmsg_batch || return $?

		update_rec_file || return $?
	done < "$trace_server_kmsg_dumper_fifo" &

	trace_server_kmsg_dumper_pids+="$!"

	trace_kmsg_dumper > "$trace_server_kmsg_dumper_fifo" &

	trace_server_kmsg_dumper_pids+=" $!"

	wait
}

trace_is_clear_only_op()
{
	local op=$1

	[ "$op" -eq $TRACE_OP_CLEAR ]
}

trace_is_clear_op()
{
	local op=$1

	[ "$op" -eq $TRACE_OP_PRINT_AND_CLEAR -o \
	  "$op" -eq $TRACE_OP_CLEAR ]
}

trace_is_print_op()
{
	local op=$1

	[ "$op" -eq $TRACE_OP_PRINT_AND_CLEAR -o \
	  "$op" -eq $TRACE_OP_PRINT ]
}

trace_is_print_or_count_op()
{
	local op=$1

	trace_is_print_op "$op" || \
		[ "$op" -eq $TRACE_OP_COUNT ]
}

trace_get_recs_locked()
{
	local trace_op=$1
	local since_rec=$2
	local max_recs=$3
	local dump_recs
	local dropped_recs
	local last_rec
	local -a ri

	read -a ri < "$TRACE_INFO_FILE" || cmd_err_ret

	last_rec=$((ri[RI_FIRST_REC] + ri[RI_NUM_RECS]))
	[ "$since_rec" -gt "$last_rec" ] && since_rec=$last_rec

	dropped_recs=${ri[RI_FIRST_REC]}

	if [ $((ri[RI_FLAGS] & TRACE_FLAG_CLEARED)) -ne 0 ]; then
		since_rec=$last_rec
		dropped_recs=$last_rec
	fi

	[ "$since_rec" -lt "${ri[RI_FIRST_REC]}" ] && \
		since_rec=${ri[RI_FIRST_REC]}

	dump_recs=$((last_rec - since_rec))
	[ $max_recs -ge 0 -a \
	  $dump_recs -gt $max_recs ] && dump_recs=$max_recs

	echo "$since_rec" "$dump_recs" "$dropped_recs"

	since_rec=$((since_rec - ri[RI_FIRST_REC]))

	# tail -n +1 is starts dumping from the first line
	if trace_is_print_or_count_op "$trace_op"; then
		tail -n +"$((since_rec + 1))" < "$TRACE_REC_FILE" | \
			head -n "$dump_recs" || cmd_err_ret
	fi

	if trace_is_clear_op "$trace_op"; then
		trace_clear_recs ri
	fi
}

trace_get_recs()
{
	local trace_op=$1
	local since_rec=$2
	local max_recs=$3

	call_with_flock "$TRACE_LOCK_FILE" \
			trace_get_recs_locked "$trace_op" "$since_rec" "$max_recs"
}

get_dmesg_prefix()
{
	local -nr __dr_pfx=$1
	local caller_id=${__dr_pfx[DR_CALLER_ID]:-}
	local time_sec
	local time_nsec
	local time

	time_sec=$((__dr_pfx[DR_TIMESTAMP] / 1000000))
	time_nsec=$((__dr_pfx[DR_TIMESTAMP] % 1000000))

	time_nsec=$(lpad_zero "$time_nsec" 6)
	time="$time_sec.$time_nsec"
	time="[$(lpad_space "$time" 13)]"

	[ -n "$caller_id" ] && caller_id="[$(lpad_space "$caller_id" 8)]"

	echo -n "${time}${caller_id}"
}

print_dmesg_line()
{
	local indent=$1
	local printed=$2
	local rec_filter_match=$3
	local -nr __dr_prn=$4

	$printed || return 0
	[ $rec_filter_match -eq $REC_FILTER_MATCH ] || return 0

	echo -e "${indent}$(get_dmesg_prefix __dr_prn)${__dr_prn[DR_MESSAGE]:+ ${__dr_prn[DR_MESSAGE]}}"
}

print_repeated_dmesg_line()
{
	local indent=$1
	local printed=$2
	local rec_filter_match=$3
	local rep_count=$4
	local -nr __dr_rep=$5
	local prefix

	$printed || return 0
	[ $rec_filter_match -eq $REC_FILTER_MATCH ] || return 0

	case "$rep_count" in
	0)
		return
		;;
	1)
		print_dmesg_line "$indent" "$printed" "$rec_filter_match" \
				 __dr_rep || return $?
		return
		;;
	esac

	prefix="${indent}$(get_dmesg_prefix __dr_rep)"

	echo -e "${prefix} --- Previous line repeated $rep_count times"
}

rec_matches_filter()
{
	local msg_config=$1
	local msg_regex=$2
	local dev_regex=$3
	local -nr __dr_flt=$4
	local pattern

	if [ -n "$dev_regex" ]; then
		if ! [[ "${__dr_flt[DR_DEVICE]}" =~ $dev_regex ]]; then
			return $REC_FILTER_UNMATCH
		fi
	fi

	if [ "$msg_config" -eq $TRACE_FILTER_NONE ]; then
		return $REC_FILTER_MATCH
	fi

	if [[ "${__dr_flt[DR_MESSAGE]}" =~ $msg_regex ]]; then
		[ "$msg_config" -eq $TRACE_FILTER_ONLY ] && \
			return $REC_FILTER_MATCH
	else
		[ "$msg_config" -eq $TRACE_FILTER_EXCLUDE ] && \
			return $REC_FILTER_MATCH
	fi

	return $REC_FILTER_UNMATCH
}

parse_dmesg_rec()
{
	local line=$1
	local -a dr
	local header cont

	header=${line%%;*}
	line=${line#*;}

	cont=${header%%,*}
	header=${header#*,}
	cont=${cont# }

	fld=${cont%% *}
	dr[DR_SUBSYSTEM]=${fld#SUBSYSTEM=}
	[ "${dr[DR_SUBSYSTEM]}" = "$fld" ] && \
		dr[DR_SUBSYSTEM]=""

	fld=${cont#* }
	dr[DR_DEVICE]=${fld#DEVICE=}
	[ "${dr[DR_DEVICE]}" = "$fld" ] && \
		dr[DR_DEVICE]=""

	dr[DR_PRIO]=${header%%,*}
	header=${header#*,}

	dr[DR_SEQ]=${header%%,*}
	header=${header#*,}

	dr[DR_TIMESTAMP]=${header%%,*}
	header=${header#*,}

	fld=${header%%,*}   # options TODO: line continuation
	header=${header#*,}

	fld=${header%%,*}
	dr[DR_CALLER_ID]=${fld#caller=}
	[ "${dr[DR_CALLER_ID]}" = "$fld" ] && \
		dr[DR_CALLER_ID]=""

	dr[DR_MESSAGE]=""

	IFS=,
	echo "${dr[*]};$line"
	IFS=$IFS_STD
}

parse_dmesg_rec_process()
{
	local line

	while read -r line; do
		parse_dmesg_rec "$line"
	done
}

drecs_match()
{
	local -nr dr1=$1
	local -nr dr2=$2

	[ "$((dr1[DR_SEQ] + 1))"   = "${dr2[DR_SEQ]}" -a \
	  "${dr1[DR_SUBSYSTEM]:-}" = "${dr2[DR_SUBSYSTEM]:-}" -a \
	  "${dr1[DR_DEVICE]:-}"    = "${dr2[DR_DEVICE]:-}" -a \
	  "${dr1[DR_PRIO]}"        = "${dr2[DR_PRIO]}" -a \
	  "${dr1[DR_CALLER_ID]:-}" = "${dr2[DR_CALLER_ID]:-}" -a \
	  "${dr1[DR_MESSAGE]:-}"   = "${dr2[DR_MESSAGE]:-}" ]
}

get_dr()
{
	local dr_line=$1
	local -n __dr_get=$2
	local header=${dr_line%%;*}
	local message=${dr_line#*;}

	IFS=, read -a __dr_get < <(echo "$header") || cmd_err_ret

	__dr_get[DR_MESSAGE]=$message
}

trace_dump_recs()
{
	local trace_op=$1
	local filter_msg_config=$2
	local filter_msg_regex=$3
	local filter_dev_regex=$4
	local since_rec=$5
	local max_recs=$6
	local indent=$(repeat_char " " $7)
	local first_rec num_recs dropped_recs
	local lines
	local dr_line
	local -a dr
	local -a dr_last=("" "" "" "" "" "" "")
	local line
	local last_line=""
	local rep_count=0
	local printed=false
	local rec_filter_match=$REC_FILTER_UNMATCH
	local -a rec_count=(0 0)
	local err=0

	lines=$(trace_get_recs "$trace_op" \
			       "$since_rec" "$max_recs") || return $?

	trace_is_clear_only_op "$trace_op" && return 0
	trace_is_print_op "$trace_op" && printed=true

	read -r first_rec num_recs dropped_recs \
		< <(echo "$lines" | head -1) || cmd_err_ret

	if $printed && [ $dropped_recs -gt 0 ]; then
		echo "${indent}Dropped/cleared $dropped_recs records"
	fi

	dr_last_set=0
	while IFS= read dr_line; do
		dr_last_set=$((dr_last_set + 1))

		if [ $err -ne 0 ] || test_state_must_break; then
			break
		fi

		get_dr "$dr_line" dr || return $?

		if drecs_match dr_last dr; then
			dr_last[DR_SEQ]=${dr[DR_SEQ]}
			dr_last[DR_TIMESTAMP]=${dr[DR_TIMESTAMP]}
			rep_count=$((rep_count + 1))

			continue
		fi

		rec_count[rec_filter_match]=$((rec_count[rec_filter_match] + rep_count))

		print_repeated_dmesg_line "$indent" "$printed" \
					  "$rec_filter_match" "$rep_count" \
					  dr_last || err=$?

		[ $err -ne 0 -a $err -ne $ERR_INT ] && return $err

		rep_count=0
		dr_last=("${dr[@]}")

		rec_filter_match=$REC_FILTER_MATCH
		if ! rec_matches_filter "$filter_msg_config" \
					"$filter_msg_regex" \
					"$filter_dev_regex" \
					dr; then
			rec_filter_match=$REC_FILTER_UNMATCH
		fi

		rec_count[rec_filter_match]=$((rec_count[rec_filter_match] + 1))

		print_dmesg_line "$indent" "$printed" \
				 "$rec_filter_match" \
				 dr || err=$?
		[ $err -ne 0 -a $err -ne $ERR_INT ] && return $err
	done < <(echo "$lines" | stdbuf -oL tail -n +2 | parse_dmesg_rec_process)

	print_repeated_dmesg_line "$indent" "$printed" \
				  "$rec_filter_match" "$rep_count" \
				  dr_last || err=$?
	[ $err -ne 0 -a $err -ne $ERR_INT ] && return $err

	rec_count[rec_filter_match]=$((rec_count[rec_filter_match] + rep_count))

	rc[RC_FIRST_REC]=$first_rec
	rc[RC_NUM_RECS]=$num_recs
	rc[RC_DROPPED_RECS]=$dropped_recs
	rc[RC_MATCH_RECS]=${rec_count[REC_FILTER_MATCH]}

	rc[RC_LAST_SEQ]=${dr_last[DR_SEQ]:-0}
	rc[RC_LAST_TIMESTAMP]=${dr_last[DR_TIMESTAMP]:-0}

	$printed || echo "${rc[@]}"

	test_state_must_break && err=$ERR_INT

	return $err
}

trace_init()
{
	local -a ri=(0 0 0 0)
	local -a di=(-1 -1 -1)

	TRACE_TMP_DIR=$(mktemp -d) || cmd_err_ret

	debug "Temp dir: $TRACE_TMP_DIR\n"

	KMSG_DUMPER_LOCK_FILE="$TRACE_TMP_DIR/dumper-lock"
	KMSG_DUMPER_SEEK_FILE="$TRACE_TMP_DIR/dumper-seek"
	KMSG_DUMPER_FEED_FILE="$TRACE_TMP_DIR/dumper-feed"

	trace_files+=("$KMSG_DUMPER_FEED_FILE")
	trace_files+=("$KMSG_DUMPER_SEEK_FILE")
	trace_files+=("$KMSG_DUMPER_LOCK_FILE")

	TRACE_LOCK_FILE="$TRACE_TMP_DIR/trace-lock"
	TRACE_INFO_FILE="$TRACE_TMP_DIR/trace-info"
	TRACE_REC_FILE="$TRACE_TMP_DIR/trace-recs"

	trace_files+=("$TRACE_REC_FILE")
	trace_files+=("$TRACE_INFO_FILE")
	trace_files+=("$TRACE_LOCK_FILE")

	touch "${trace_files[@]}" || cmd_err_ret

	exec {TRACE_REC_IN_FD}< "$TRACE_REC_FILE" || cmd_err_ret
	exec {TRACE_REC_OUT_FD}> "$TRACE_REC_FILE" || cmd_err_ret

	exec {TRACE_INFO_IN_FD}< "$TRACE_INFO_FILE" || cmd_err_ret
	exec {TRACE_INFO_OUT_FD}> "$TRACE_INFO_FILE" || cmd_err_ret

	trace_write_dumper_info "$KMSG_DUMPER_SEEK_FILE" di || return $?
	trace_write_dumper_info "$KMSG_DUMPER_FEED_FILE" di || return $?
	trace_write_info ri || return $?

	trace_server &
	trace_server_pid=$!

	return 0
}

trace_cleanup()
{
	local tmp_dir

	debug "Trace cleanup: $trace_server_pid\n"

	if [ -n "$trace_server_pid" ]; then
		"$KILL" $trace_server_pid 2> /dev/null || true
		wait "$trace_server_pid" 2> /dev/null || true
	fi

	[ -n "${TRACE_INFO_OUT_FD+x}" ] && exec {TRACE_INFO_OUT_FD}>&- || true
	[ -n "${TRACE_INFO_IN_FD+x}" ] && exec {TRACE_INFO_IN_FD}>&- || true
	[ -n "${TRACE_REC_OUT_FD+x}" ] && exec {TRACE_REC_OUT_FD}>&- || true
	[ -n "${TRACE_REC_IN_FD+x}" ] && exec {TRACE_REC_IN_FD}>&- || true

	"$RM" -f "${trace_files[@]}" || true

	"$RMDIR" "$TRACE_TMP_DIR" || true
}

shopt -s extglob

COLOR_RED=""
COLOR_GREEN=""
COLOR_PURPLE=""
COLOR_LIGHT_GREY=""
COLOR_YELLOW=""
COLOR_GREY=""
COLOR_NONE=""

declare -r USEC_PER_SEC=1000000

declare -r SIG_HUP=1
declare -r SIG_INT=2
declare -r SIG_QUIT=3
declare -r SIG_ILL=4
declare -r SIG_TRAP=5
declare -r SIG_ABRT=6
declare -r SIG_BUS=7
declare -r SIG_FPE=8
declare -r SIG_KILL=9
declare -r SIG_USR1=10
declare -r SIG_SEGV=11
declare -r SIG_USR2=12
declare -r SIG_PIPE=13
declare -r SIG_ALRM=14
declare -r SIG_TERM=15
declare -r SIG_STKFLT=16
declare -r SIG_CHLD=17
declare -r SIG_CONT=18
declare -r SIG_STOP=19
declare -r SIG_TSTP=20
declare -r SIG_TTIN=21
declare -r SIG_TTOU=22
declare -r SIG_URG=23
declare -r SIG_XCPU=24
declare -r SIG_XFSZ=25
declare -r SIG_VTALRM=26
declare -r SIG_PROF=27
declare -r SIG_WINCH=28
declare -r SIG_IO=29
declare -r SIG_PWR=30
declare -r SIG_SYS=31

declare -r SIG_RTMIN=34
declare -r SIG_RTMAX=64

declare -r ERR_NOEXEC=126
declare -r ERR_RESERVED_START=$ERR_NOEXEC
declare -r ERR_NOENT=127
declare -r ERR_SIGNAL=128

sys_err_signal()
{
	echo "$((ERR_SIGNAL + $1))"
}

# Convenience for common errors
declare -r ERR_INT=$(sys_err_signal $SIG_INT)
declare -r ERR_ALRM=$(sys_err_signal $SIG_ALRM)

declare -r ERR_SCRIPT_START=240
declare -r ERR_CMD_GENERIC=$((ERR_SCRIPT_START - 1))

declare -r IFS_STD=$' \t\n'

declare -r ASCII_ESC=$'\33'
declare -r ASCII_ESC_SEQ="${ASCII_ESC}["

declare -r MIN_BASH_MAJOR=4
declare -r MIN_BASH_MINOR=1

declare -r SYSFS_DIR=/sys
declare -r SYSFS_PCI_DEV_DIR=$SYSFS_DIR/bus/pci/devices
declare -r SYSFS_DRM_CLASS_DIR=$SYSFS_DIR/class/drm

declare -r INTEL_ALIAS_VENDOR_ID="0x8087"
declare -r INTEL_VENDOR_ID="0x8086"

declare -r TEST_STATE_RUN=0
declare -r TEST_STATE_PAUSE=1
declare -r TEST_STATE_BREAK=2
declare -r TEST_STATE_QUIT=3

declare -A test_state_names=(
	[$TEST_STATE_RUN]="run"
	[$TEST_STATE_PAUSE]="pause"
	[$TEST_STATE_BREAK]="break"
	[$TEST_STATE_QUIT]="quit"
)

test_state=$TEST_STATE_RUN

logger_at_line_start=true

debug_enabled=false
debug_file=

utf8_supported=false

# Reserve error codes starting from ERR_SCRIPT_START to script errors.
# If commands return an error code in this range (unlikely?) fold these
# to ERR_CMD_GENERIC.
to_err() {
	local err=$?

	[ $err -gt $ERR_CMD_GENERIC ] && err=$ERR_CMD_GENERIC

	echo "$err"
}

is_cmd_err() {
	[ $1 -lt $ERR_SCRIPT_START ]
}

bit()
{
	local bit_pos=$1

	echo "$((1 << bit_pos))"
}

min()
{
	local a=$1
	local b=$2

	if [ $a -lt $b ]; then
		echo "$a"
		return
	fi

	echo "$b"
}

max()
{
	local a=$1
	local b=$2

	if [ $a -gt $b ]; then
		echo "$a"
		return
	fi

	echo "$b"
}

debug()
{
	$debug_enabled || return 0

	if [ -n "${debug_file+x}" ]; then
		echo -ne "$@" >> "$debug_file"
	else
		echo -ne "$@" >&2
	fi

	return 0
}

enable_debug()
{
	local file=${1-}

	debug_enabled=true
	debug_file="$file"
}

debug_is_enabled()
{
	[ -n "${debug_enabled+x}" ] || return 1

	$debug_enabled
}

epoch_usec()
{
	local time=$EPOCHREALTIME
	local sec usec

	sec="${time/+([^0-9])*/}"
	usec="${time/+([0-9])+([^0-9])?(+(0))/}"

	[ -z "$usec" ] && usec=0

	echo $((sec * 1000000 + usec))
}

debug_delta_start()
{
	local label=$1

	debug "$label: start\n"

	echo $(epoch_usec)
}

debug_delta_end()
{
	local label=$1
	local start=$2
	local end=$(epoch_usec)
	local delta
	local txt=""

	[ -z "$start" ] && return

	delta=$((end - start))

	if [ $delta -gt 1000000 ]; then
		txt="$((delta / 1000000))s "
	fi

	txt+="$((delta % 1000000))u"

	debug "$label: end +$txt\n"

	echo "$end"
}

pr_err()
{
	echo -ne "$@" >&2
}

err_exit()
{
	pr_err -ne "$@"
	exit 1
}

check_bash_version()
{
	local major minor trail

	major=${BASH_VERSION%%.*}
	trail=${BASH_VERSION#*.}
	minor=${trail%%.*}

	if [ "$major" -lt "$MIN_BASH_MAJOR" -o \
	     \( "$major" -eq "$MIN_BASH_MAJOR" -a "$minor" -lt "$MIN_BASH_MINOR" \) ]; then
		err_exit "Bash version too old:$BASH_VERSION (required at least $MIN_BASH_MAJOR.$MIN_BASH_MINOR)\n"
	fi
}

valid_number()
{
	local number=$1

	[ -z "${number}" -o -n "${number/#+([0-9])/}" ] && return 1

	return 0
}

sec_to_time()
{
	local sec=$1
	local str=""

	if [ $sec -ge $((60 * 60)) ]; then
		str+="$((sec / 60 / 60))h"
		sec=$((sec % (60 * 60)))
	fi

	if [ -n "$str" -o $sec -ge 60 ]; then
		str+="$((sec / 60))m"
		sec=$((sec % 60))
	fi

	str+="${sec}s"

	echo "$str"
}

sec_to_date_time()
{
	local seconds=$1

	"$DATE" +"%Y-%m-%d %H:%M:%S" -d @"$seconds"
}

get_uptime_sec()
{
	local uptime=$(< /proc/uptime)

	echo "${uptime%%.*}"
}

msec_to_sec()
{
	local ms=$1

	echo $((ms / 1000)).$((ms % 1000))
}

unbuffered_echo()
{
	"$STDBUF" -oL echo "$@"
}

log_cont()
{
	local line=$1
	local start_nl=false
	local end_nl=false

	if [ "${line:0:2}" = '\n' ]; then
		start_nl=true
		line=${line:2}
	fi

	if [ "${line: -2}" = '\n' -o "${line: -2}" = '\r' ]; then
		end_nl=true
	fi

	if $start_nl && ! $logger_at_line_start; then
		unbuffered_echo
	fi

	unbuffered_echo -ne "$line"

	logger_at_line_start=$end_nl
}

log()
{
	log_cont "\n$@" >&2
}

log_err()
{
	log "${COLOR_RED}$@${COLOR_NONE}" >&2
}

log_cr()
{
	log_cont "\r" >&2
}

char_to_int()
{
	local char=$1

	"$PRINTF" "%d" "'$char"
}

int_to_char()
{
	local int=$1

	echo -e "\x$("$PRINTF" "%x" $int)"
}

is_ctrl_char_code()
{
	local code=$1

	[ $code -eq 127 ] && return 0
	[ $code -lt 32 ] && return 0

	return 1
}

ctrl_char_code_ascii_code()
{
	local code=$1

	if [ $code -eq 127 ]; then
		echo -- "-1"
	fi

	echo "$code"
}

int_to_ascii_ctrl_seq()
{
	local int=$1

	echo "^$(int_to_char "$(($base + $int))")"
}

repeat_char()
{
	local c=$1
	local n=$2
	local str=""

	for ((i = 0; i < $n; i++)); do
		str+="$c"
	done

	echo -n "$str"
}

declare -r pad_spaces="                                                      "
declare -r pad_zeroes="000000000000000000000000000000000000000000000000000000"

lpad_space()
{
	local str=$1
	local n=$2

	echo "${pad_spaces:0:$((n - ${#str}))}$str"
}

rpad_space()
{
	local str=$1
	local n=$2

	echo "$str${pad_spaces:0:$((n - ${#str}))}"
}

lpad_zero()
{
	local str=$1
	local n=$2

	echo "${pad_zeroes:0:$((n - ${#str}))}$str"
}

emph_first()
{
	local str=$1

	echo "${COLOR_YELLOW}${str:0:1}${COLOR_NONE}${str:1}"
}

emph_paren_first()
{
	local str=$1

	echo "(${COLOR_YELLOW}${str:0:1}${COLOR_NONE})${str:1}"
}

is_complete_esc_seq()
{
	local seq=$1
	local len=${#seq}
	local last_char=${seq: -1}

	case $len in
	1)
		[ "$last_char" != "$ASCII_ESC" ] && return 0
		;;
	2)
		[ "$last_char" != "[" ] && return 0
		;;
	*)
		[ $len -ge 32 ] && return 0   # a random limit
		[[ "$last_char" =~ [A-Za-z~] ]] && return 0
	esac

	return 1
}

append_esc_seq()
{
	local seq=$1
	local char=$2

	if [ -n "$esc_seq" ]; then
		esc_seq+="$char"

		if is_complete_esc_seq "$esc_seq"; then
			esc_seq=""
		fi

		echo "$esc_seq"

		return 0
	fi

	if [ "$char" = "$ASCII_ESC" ]; then
		esc_seq=$char

		echo "$esc_seq"

		return 0
	fi

	echo ""

	return 1
}

get_str_len()
{
	local str=$(echo -e "$1")
	local esc_seq=""
	local str_len=0
	local char_len=1
	local i

	for ((i = 0; i < "${#str}"; i += char_len)) {
		local char=${str:i:1}

		char_len=1
		esc_seq=$(append_esc_seq "$esc_seq" "$char") && continue

		str_len=$((str_len + 1))
	}

	echo "$str_len"
}

cursor_move_back()
{
	local n=$1
	local back

	back=$(repeat_char "\b" $n) || return $?

	[ $n -gt 0 ] || return

	echo -ne "$back"
}

cursor_erase_back()
{
	local n=$1
	local spaces

	spaces=$(repeat_char " " $n) || return $?

	[ $n -gt 0 ] || return

	cursor_move_back $n || return $?
	echo -ne "$spaces"
	cursor_move_back $n || return $?
}

cursor_update_back()
{
	local old_len=$1
	local new_len=$2
	local erase_len=$((old_len - new_len))

	[ $erase_len -lt 0 ] && erase_len=0

	cursor_erase_back $erase_len
	cursor_move_back $((old_len - erase_len))
}

update_back_if_changed()
{
	local old_str=$1
	local new_str=$2
	local old_len
	local new_len

	old_len=$(get_str_len "$old_str") || return $?
	new_len=$(get_str_len "$new_str") || return $?

	[ ${#old_str} -eq ${#new_str} ] && return

	cursor_update_back $old_len $new_len
}

cursor_erase_str()
{
	local str=$1
	local len=$(get_str_len "$str")

	cursor_erase_back "$(get_str_len "$str")"
}

escape_char_to_ascii_notation()
{
	local char=$1
	local char_code=$(char_to_int "$char")
	local base=$(char_to_int "@")

	if ! is_ctrl_char_code "$char_code"; then
		echo "$char"
		return
	fi

	int_to_ascii_ctrl_seq "$char_code"
}

int_to_hex_esc_seq()
{
	local int=$1

	"$PRINTF" '\\x%02x' $int
}

escape_string_to_ascii_notation()
{
	local str=$1
	local esc_str=""
	local i

	for ((i = 0; i < ${#str}; i++)); do
		esc_str+="$(escape_char_to_ascii_notation "${str:$i:1}")"
	done

	echo "$esc_str"
}

parse_ascii_seq()
{
	local ascii_seq=$1
	local seq_table=(
		"[A:\u2191:^Up"
		"[B:\u2193:^Down"
		"[C:\u2192:^Right"
		"[D:\u2190:^Left"
		"[5~:\u21D1:^PgUp"
		"[6~:\u21D3:^PgDown"
		""
	)
	local seq_ascii seq_unicode seq_notation

	for seq in "${seq_table[@]}"; do
		[ -z "$seq" ] && break
		IFS=: read -r seq_ascii seq_unicode seq_notation < <(echo "$seq")
		[ "$seq_ascii" != "${ascii_seq:1}" ] || break
	done

	if [ -z "$seq" ]; then
		echo "$(escape_string_to_ascii_notation ${ascii_seq})"
		return
	fi

	if $utf8_supported; then
		echo -e "$seq_unicode"
		return
	fi

	echo "$seq_notation"
}

get_symbol_for_char()
{
	local seq=$1

	if [ "${seq:0:1}" = $ASCII_ESC ]; then
		parse_ascii_seq "$seq"
		return
	fi

	echo "$(escape_string_to_ascii_notation ${seq})"
}

read_ascii_esc_seq()
{
	local char
	local seq="$ASCII_ESC"

	while true; do
		if ! read -r -n 1 -t 0.1 char; then
			break
		fi
		seq="${seq}${char}"

		is_complete_esc_seq "$seq" && break
	done

	echo "$seq"
}

read_char()
{
	local timeout=$1
	local char=""
	local tmo_opt="-t $timeout"
	local ascii_seq
	local ret=0

	if [ "$timeout" = "-1" ]; then
		tmo_opt=""
	fi

	read -n 1 -r $tmo_opt char || ret=$?

	[ $ret -ne 0 ] && return 1

	if [ "$char" = $ASCII_ESC ]; then
		char="$(read_ascii_esc_seq)"
	fi

	echo -n "$char"

	return 0
}

flush_input()
{
	local char

	while char=$(read_char 0.1); do
		:
	done
}

plural_str()
{
	local num=$1
	local str=$2

	[ $num -gt 1 ] && str+="s"

	echo "$str"
}

shell_esc()
{
	local str=$1

	str=${str//\\/\\\\}
	str="${str//\"/\\\"}"
	str="${str//$/\\$}"

	echo "$str"
}

script_param_inst()
{
	local str=$1

	str=${str//\{\}/\"}

	echo "$str"
}

test_state_is()
{
	local state=$1

	[ $test_state -eq $state ]
}

test_state_is_run()
{
	test_state_is $TEST_STATE_RUN
}

test_state_must_break()
{
	[ $test_state -ge $TEST_STATE_BREAK ]
}

test_state_must_quit()
{
	[ $test_state -ge $TEST_STATE_QUIT ]
}

test_state_set()
{
	test_state_must_quit && return  # refuse changing from quit
	test_state=$1
}

test_state_name()
{
	local state=$1

	echo "${test_state_names[$state]}"
}


get_ssh_server_address()
{
	local ip_address
	local ip_octet
	local ip_port
	local i
	local -r address_regex="^([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3})\.([0-9]{1,3}) ([0-9]{1,6})\b"

	if ! [ -v SSH_CONNECTION ]; then
		pr_err "The SSH_CONNECTION environment variable is not set.\n"
		return 1
	fi

	if ! [[ $SSH_CONNECTION =~ $address_regex ]]; then
		pr_err "Malformed IP address/port in SSH_CONNECTION variable\n"
		return 1
	fi

	for ((i=1; i<=4; i++)); do
		ip_octet=${BASH_REMATCH[i]}

		if [[ $ip_octet -gt 255 ]]; then
			pr_err "Invalid IP address octet \"$ip_octet\" in SSH_CONNECTION variable\n"
			return 1
		fi

		ip_address+="${ip_address:+.}$ip_octet"
	done

	ip_port=${BASH_REMATCH[5]}
	if [ $ip_port -lt 1024 ]; then
		pr_err "Invalid SSH client IP port \"$ip_port\"\n"
		return 1
	fi

	echo "$ip_address:$ip_port"
}

cache_sudo_right()
{
	[ "$(id -u)" -eq 0 ] && return 0

	[[ -v SUDO ]] || err_exit "SUDO tool is not initialized\n"

	$SUDO true || return $?
}


init_raw_term()
{
	saved_stty_config=$("$STTY" -g)
	"$STTY" -raw -echoctl -echo
}

cleanup_raw_term()
{
	# explicit input, accounting for calls from a signal interrupt
	# handler at a time where stdin was redirected (for instance an
	# error interrupting a pending read < $file).
	# saved_stty_config could be unset if a signal triggered before
	# init_raw_term()
	[ -z "${saved_stty_config+x}" ] && return 0
	"$STTY" "$saved_stty_config" < /dev/tty
}

is_modern_terminal()
{
	case "$TERM" in
	linux | xterm*)
		return 0
	esac

	return 1
}

term_supports_colors()
{
	# is stdout a terminal?
	[ -t 1 ] || return 1

	is_modern_terminal && return 0

	[ -v TPUT ] || return 1

	if [ $(tput colors) -gt 1 ]; then
		return 0
	fi

	return 1
}

color_esc_seq()
{
	local color_code1=$1
	local color_code2=${2:-}

	echo "${ASCII_ESC_SEQ}$color_code1${2+;}${color_code2}m"
}

init_colors()
{
	term_supports_colors || return 0

	COLOR_RED=$(color_esc_seq 0 31)
	COLOR_GREEN=$(color_esc_seq 0 32)
	COLOR_PURPLE=$(color_esc_seq 1 35)
	COLOR_LIGHT_GREY=$(color_esc_seq 37)
	COLOR_YELLOW=$(color_esc_seq 1 33)
	COLOR_GREY=$(color_esc_seq 0 90)
	COLOR_NONE=$(color_esc_seq 0)
}

init_locale()
{
	local lc_ctype

	[ -z "${LOCALE+x}" ] && return 0

	lc_ctype=$("$LOCALE" | "$GREP" LC_CTYPE) || return 0

	lc_ctype=${lc_ctype%\"}

	[ "${lc_ctype%UTF-8}" != "$lc_ctype" ] && utf8_supported=true

	return 0
}

get_columns()
{
	local size=$("$STTY" size)

	echo "${size#* }"
}

sanitize_pci_id_component()
{
	local id=$1

	id="0x${id#0x}"
	id=${id,,}

	echo "$id"
}

sanitize_pci_vendor_id()
{
	local vendor=$(sanitize_pci_id_component "$1")

	get_canonical_intel_vendor_id "$vendor"
}

sanitize_pci_device_id()
{
	sanitize_pci_id_component "$1"
}

get_pci_dev_address()
{
	local vendor=$1
	local device=$2
	local dir

	for dir in $SYSFS_PCI_DEV_DIR/*; do
		local iter_vendor iter_device

		iter_vendor=$(sanitize_pci_vendor_id $(cat "$dir"/vendor))
		iter_device=$(sanitize_pci_device_id $(cat "$dir"/device))

		[ "$iter_vendor" = "$vendor" ] || continue
		[ "$iter_device" = "$device" ] || continue

		echo $(basename "$dir")

		return 0
	done

	return 1
}

call_with_flock()
{
	local lock_file=$1
	local fn=$2

	shift 2

	(
		flock "$fd" || exit 1
		$fn "$@"
	) {fd}> "$lock_file"
}

init_utils()
{
	check_bash_version

	if [ $(get_columns) -lt 60 ]; then
		pr_err "Terminal too narrow\n"
		return 1
	fi

	# Move the following out from the library
	init_raw_term || return $?
	init_colors || return $?
	init_locale || return $?

	return 0
}

cleanup_utils()
{
	cleanup_raw_term
}

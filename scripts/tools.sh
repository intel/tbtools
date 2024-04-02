tool_cmd()
{
	echo "$(basename "$1")"
}

find_tool()
{
	PATH=${TOOL_PATH:+$TOOL_PATH:}$PATH which "$1" || return 2
}

assign_tool_path()
{
	local tool_varname=$1
	local tool_path=$2

	[ -f "$tool_path" ] || return 2
	[ -x "$tool_path" ] || err_exit "Tool $tool_path is not executable"

	eval "$tool_varname=\"$tool_path\""
}

tool_name()
{
	echo "$(tool_cmd "${1,,}")"
}

find_and_assign_tool()
{
	local tool_varname=$1
	local tool_name=$(tool_name "$tool_varname")
	local tool_path
	local err

	tool_path=$(find_tool "$tool_name")
	err=$?
	[ $err -ne 0 ] && return $err

	assign_tool_path "$tool_varname" "$tool_path"
}

init_sudo()
{
	if [ "$(id -u)" -eq 0 ]; then
		SUDO=""
		return 0
	fi

	[[ -v SUDO ]] && return 0

	pr_err "Cannot find tool sudo, required to acquire root rights\n"

	return 1
}

init_tools()
{
	local -nr tools=$1
	local -r tools_are_optional=$2
	local tool_varname

	for tool_varname in "${tools[@]}"; do
		find_and_assign_tool "$tool_varname" && continue

		$tools_are_optional && continue

		pr_err "Cannot find tool \"$(tool_name $tool_varname)\"\n"

		return 1
	done
}

__test_cmd()
{
	local errout=$1
	local err_file
	local cmd_out
	local err

	shift

	err_file=$($MKTEMP)

	if $errout; then
		cmd_out=$("$@" 2>&1)
	else
		cmd_out=$("$@" 2> "$err_file")
	fi
	err=$?

	case "$err" in
	0)
		echo "$cmd_out"
		;;
	$ERR_INT)
		log_note "Command \"$*\" was interrupted\n"
		;;
	*)
		log_err "Command \"$*\" failed with error code $err:"
		if $errout; then
			log "$cmd_out\n"
		else
			log "$(cat "$err_file")\n"
		fi
		;;
	esac

	$RM "$err_file"

	return $err
}

test_cmd()
{
	__test_cmd false "$@"
}

test_cmd_errout()
{
	__test_cmd true "$@"
}

test_cmd_no_out()
{
	test_cmd "$@" > /dev/null
}

test_cmd_retry()
{
	local first_err=0
	local err
	local i

	for ((i = 0; i < $MAX_TEST_CMD_RETRY_ATTEMPTS; i++)); do
		test_cmd "$@"
		err=$?

		[ $first_err -eq 0 ] && first_err=$err

		[ $err -ne $ERR_INT ] && break
	done

	return $first_err
}

test_cmd_retry_no_out()
{
	test_cmd_retry "$@" > /dev/null
}


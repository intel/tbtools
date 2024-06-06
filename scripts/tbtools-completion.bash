# Bash completions for tbtools.

# Enable debugging by uncommenting the below
#set -x

shopt -s extglob

_tbtools_domain_route_adapter()
{
	local arg

	for ((i=1; i < ${COMP_CWORD} - 1; i=i+1)); do
		arg="${COMP_WORDS[i]}"
		if [[ $arg = "-d" ]]; then
			domain=${COMP_WORDS[i + 1]}
		elif [[ $arg = "-r" ]]; then
			route=${COMP_WORDS[i + 1]}
		elif [[ $arg = "-a" ]]; then
			adapter=${COMP_WORDS[i + 1]}
		fi
	done
}

_tbtools_complete_domains()
{
	local domains

	domains=$(tblist -SA 2> /dev/null |
		sed 1d |
		awk -F, '$9 ~ /^Domain$/ { print $1 }' |
		xargs)
	COMPREPLY+=($(compgen -W "$domains" -- "$cur"))
}

# 1: domain (or default 0 is used)
_tbtools_complete_routers()
{
	local domain routers

	domain=${1:-0}
	routers=$(tblist -SA 2> /dev/null |
		sed -e 1d -e 's/"\([^",]\+\),*\([^"]*\)"/\1\2/' |
		awk -F, -v domain=$domain '$1 ~ domain && $9 ~ /^Router$/ { print $2 }' |
		xargs)
	COMPREPLY+=($(compgen -W "$routers" -- "$cur"))
}

# 1: route
# 2: domain (or default 0 is used)
_tbtools_complete_all_adapters()
{
	local route domain adapters

	[[ ! $1 ]] && return
	route=$1

	domain=${2:-0}
	adapters=$(tbadapters -d $domain -r $route -S 2> /dev/null |
		sed 1d |
		cut -d, -f1 |
		xargs)
	COMPREPLY+=($(compgen -W "$adapters" -- "$cur"))
}

# 1: route
# 2: domain (or default 0 is used)
_tbtools_complete_lane_adapters()
{
	local route domain adapters

	[[ ! $1 ]] && return
	route=$1

	domain=${2:-0}
	adapters=$(tbadapters -d $domain -r $route -S 2> /dev/null |
		sed 1d |
		awk -F, '$2 ~ /Lane/ { print $1 }' |
		xargs)
	COMPREPLY+=($(compgen -W "$adapters" -- "$cur"))
}

_tbtools_complete_registers()
{
	local registers

	_tbtools_domain_route_adapter
	domain=${domain:-0}
	[[ ! $route ]] && return

	# Get rid of escaped spaces
	reg="${cur//\\ / }"

	if [[ $adapter ]]; then
		registers=$(tbget -d $domain -r $route -a $adapter -Q "$reg" 2> /dev/null)
	else
		registers=$(tbget -d $domain -r $route -Q "$cur" 2> /dev/null)
	fi

	# Escape spaces with backslash
	registers="${registers// /\\\\ }"
	reg="${cur// /\\\\ }"

	local IFS=$'\n'
	COMPREPLY+=($(compgen -W "$registers" -- "$reg"))
}

_tbtools_complete()
{
	local cmd cur prev domain route adapter

	cmd="${COMP_WORDS[0]}"
	cur="${COMP_WORDS[COMP_CWORD]}"
	prev="${COMP_WORDS[COMP_CWORD-1]}"
	COMPREPLY=()

	case ${prev} in
		-d | --domain)
			_tbtools_complete_domains
			return
			;;
		-r | --route)
			_tbtools_domain_route_adapter
			_tbtools_complete_routers $domain
			return
			;;
		-a | --adapter)
			_tbtools_domain_route_adapter
			if [[ $cmd == "tbmargin" ]] || [[ $cmd == "tbpd" ]]; then
				_tbtools_complete_lane_adapters $route $domain
			else
				_tbtools_complete_all_adapters $route $domain
			fi
			return
			;;
		-m | --mode)
			if [[ $cmd == "tbpd" ]]; then
				COMPREPLY+=($(compgen -W "safe usb usb4 display-port thunderbolt" -- "$cur"))
			fi
			return
			;;
	esac

	# Only following commands support register completion
	case ${cmd} in
		tbdump)
			case ${cur} in
				[A-Za-z]*([A-Za-z0-9_]))
					_tbtools_complete_registers
					return
					;;
			esac
			;;

		tbget | tbset)
			case ${cur} in
				# Accepts fields too with '.'
				[A-Za-z]*([A-Za-z0-9_. \\]))
					_tbtools_complete_registers
					return
					;;
			esac
			;;

		*)
			;;
	esac
}

complete -F _tbtools_complete tbauth
complete -F _tbtools_complete tbadapters
complete -F _tbtools_complete tbget
complete -F _tbtools_complete tbset
complete -F _tbtools_complete tbdump
complete -F _tbtools_complete tbpd
complete -F _tbtools_complete tbmargin

_tbtrace_complete()
{
	local cmd cur prev

	cmd="${COMP_WORDS[0]}"
	cur="${COMP_WORDS[COMP_CWORD]}"
	prev="${COMP_WORDS[COMP_CWORD-1]}"
	COMPREPLY=()

	case ${prev} in
		-d | --domain)
			if [[ ${COMP_WORDS[1]} == "enable" ]]; then
				_tbtools_complete_domains
			fi
			return
			;;
		-i | --input)
			if [[ ${COMP_WORDS[1]} == "dump" ]]; then
				COMPREPLY+=($(compgen -f -- "$cur"))
			fi
			return
			;;
		-*)
			return
			;;
		enable)
			return
			;;
		status | disable | dump | clear | help)
			return
			;;
		*)
			COMPREPLY+=($(compgen -W "status enable disable dump clear help" -- "$cur"))
			return
			;;
	esac
}

complete -F _tbtrace_complete tbtrace

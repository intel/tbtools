#                                                          -*- shell-script -*-
# Bash completion for tbtools.
#
# Depends on bash-completion scripts typically installed with
# "bash-completion" package:
#
# https://github.com/scop/bash-completion/
#
# The way to install this is to copy it to /usr/share/bash-completion/completions/
# and then create symlinks for all the tools you are interested in:
#
# cd /usr/share/bash-completion/completions/
# ln -s tbtools-completion.bash tbadapters
# ln -s tbtools-completion.bash tbauth
# ln -s tbtools-completion.bash tbdump
# ln -s tbtools-completion.bash tbget
# ln -s tbtools-completion.bash tblist
# ln -s tbtools-completion.bash tbmargin
# ln -s tbtools-completion.bash tbpd
# ln -s tbtools-completion.bash tbset
# ln -s tbtools-completion.bash tbtrace
#

_tbtools_domain_route_adapter()
{
    local arg

    domain=
    route=
    adapter=
    path=
    counters=
    for ((i=1; i < ${COMP_CWORD}; i=i+1)); do
        arg="${COMP_WORDS[i]}"
        if [[ $arg == '-d' || $arg == '--domain' ]]; then
            domain=${COMP_WORDS[i + 1]}
        elif [[ $arg == '-r' || $arg == '--route' ]]; then
            route=${COMP_WORDS[i + 1]}
        elif [[ $arg == '-a' || $arg == '--adapter' ]]; then
            adapter=${COMP_WORDS[i + 1]}
        elif [[ $arg == '-p' || $arg == '--path' ]]; then
            path=1
        elif [[ $arg == '-c' || $arg == '--counters' ]]; then
            counters=1
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
        # No completion for path or counter registers
        if [[ $path || $counters ]]; then
            return
        else
            registers=$(tbget -d $domain -r $route -a $adapter -Q "$reg" 2> /dev/null)
        fi
    else
        registers=$(tbget -d $domain -r $route -Q "$cur" 2> /dev/null)
    fi

    # Escape spaces with backslash
    registers="${registers// /\\\\ }"
    reg="${cur// /\\\\ }"

    local IFS=$'\n'
    COMPREPLY+=($(compgen -W "$registers" -- "$reg"))
}

_tbadapters()
{
    local cur prev words cword domain route
    _init_completion || return

    if [[ $cur == -* ]]; then
        COMPREPLY+=($(compgen -W '--domain --route --adapter --script --help
            --version' -- "$cur"))
    else
        case $prev in
            --domain | -d)
                _tbtools_complete_domains
                return
                ;;
            --route | -r)
                _tbtools_domain_route_adapter
                _tbtools_complete_routers $domain
                return
                ;;
            --adapter | -a)
                _tbtools_domain_route_adapter
                _tbtools_complete_all_adapters $route $domain
                return
                ;;
        esac
    fi
} &&
    complete -F _tbadapters tbadapters

_tbauth()
{
    local cur prev words cword domain
    _init_completion || return

    if [[ $cur == -* ]]; then
        COMPREPLY+=($(compgen -W '--add-key-path --challenge-key-path --help
            --deauthorize --domain --route --version' -- "$cur"))
    else
        case $prev in
            --add-key-path | --challenge-key-path | -[AC])
                _filedir
                return
                ;;
            --domain | -d)
                _tbtools_complete_domains
                return
                ;;
            --route | -r)
                _tbtools_domain_route_adapter
                _tbtools_complete_routers $domain
                return
                ;;
        esac
    fi
} &&
    complete -F _tbauth tbauth

_tbdump()
{
    local cur prev words cword domain route path counters
    _init_completion || return

    if [[ $cur == -* ]]; then
        COMPREPLY+=($(compgen -W '--domain --route --adapter --path --counters
            --verbose --cap-id --vs-cap-id --nregs --help' -- "$cur"))
    else
        case $prev in
            --domain | -d)
                _tbtools_complete_domains
                return
                ;;
            --route | -r)
                _tbtools_domain_route_adapter
                _tbtools_complete_routers $domain
                return
                ;;
            --adapter | -a)
                _tbtools_domain_route_adapter
                _tbtools_complete_all_adapters $route $domain
                return
                ;;
            --cap-id | --vs-cap-id | --nregs | -[CVN])
                return
                ;;
        esac
        if [[ $cur != -* ]]; then
            _tbtools_complete_registers
        fi
    fi
} &&
    complete -F _tbdump tbdump

_tbget()
{
    local cur prev words cword domain route
    _init_completion || return

    if [[ $cur == -* ]]; then
        COMPREPLY+=($(compgen -W '--domain --route --adapter --path --counters
            --binary --decimal --query --help --version' -- "$cur"))
    else
        case $prev in
            --domain | -d)
                _tbtools_complete_domains
                return
                ;;
            --route | -r)
                _tbtools_domain_route_adapter
                _tbtools_complete_routers $domain
                return
                ;;
            --adapter | -a)
                _tbtools_domain_route_adapter
                _tbtools_complete_all_adapters $route $domain
                return
                ;;
        esac
        if [[ $cur != -* ]]; then
            _tbtools_complete_registers
        fi
    fi
} &&
    complete -F _tbget tbget

_tblist()
{
    local cur prev words cword
    _init_completion || return

    if [[ $cur == -* ]]; then
        COMPREPLY+=($(compgen -W '--all --script --tree --verbose --help
            --version' -- "$cur"))
    fi
} &&
    complete -F _tblist tblist

_tbmargin()
{
    local cur prev words cword domain route
    _init_completion || return

    if [[ $cur == -* ]]; then
        COMPREPLY+=($(compgen -W '--domain --route --adapter --index --caps
            --help --version' -- "$cur"))
    else
        case $prev in
            --domain | -d)
                _tbtools_complete_domains
                return
                ;;
            --route | -r)
                _tbtools_domain_route_adapter
                _tbtools_complete_routers $domain
                return
                ;;
            --adapter | -a)
                _tbtools_domain_route_adapter
                _tbtools_complete_lane_adapters $route $domain
                return
                ;;
        esac
    fi
} &&
    complete -F _tbmargin tbmargin

_tbpd()
{
    local cur prev words cword domain route
    _init_completion || return

    if [[ $cur == -* ]]; then
        COMPREPLY+=($(compgen -W '--domain --route --adapter --mode --help
            --version' -- "$cur"))
    else
        case $prev in
            --domain | -d)
                _tbtools_complete_domains
                return
                ;;
            --route | -r)
                _tbtools_domain_route_adapter
                _tbtools_complete_routers $domain
                return
                ;;
            --adapter | -a)
                _tbtools_domain_route_adapter
                _tbtools_complete_lane_adapters $route $domain
                return
                ;;
            --mode | -m)
                COMPREPLY+=($(compgen -W "safe usb usb4 display-port thunderbolt" -- "$cur"))
                return
                ;;
        esac
    fi
} &&
    complete -F _tbpd tbpd

_tbset()
{
    local cur prev words cword domain route adapter
    _init_completion || return

    if [[ $cur == -* ]]; then
        COMPREPLY+=($(compgen -W '--domain --route --adapter --path --counters
            --help --version' -- "$cur"))
    else
        case $prev in
            --domain | -d)
                _tbtools_complete_domains
                return
                ;;
            --route | -r)
                _tbtools_domain_route_adapter
                _tbtools_complete_routers $domain
                return
                ;;
            --adapter | -a)
                _tbtools_domain_route_adapter
                _tbtools_complete_all_adapters $route $domain
                return
                ;;
        esac
        if [[ $cur != -* ]]; then
            _tbtools_complete_registers
        fi
    fi
} &&
    complete -F _tbset tbset

_tbtrace()
{
    local cur prev words cword
    _init_completion || return

    local arg
    _get_first_arg

    if [[ -z $arg ]]; then
        if [[ $cur == -* ]]; then
            COMPREPLY+=($(compgen -W '--help --version' -- "$cur"))
        else
            COMPREPLY+=($(compgen -W 'status enable disable dump clear help' -- "$cur"))
        fi
    else
        case $arg in
            dump)
                if [[ $cur == -* ]]; then
                    COMPREPLY=($(compgen -W '--help --input --script --time
                        --verbose' -- "$cur"))
                else
                    case $prev in
                        --input | -i)
                            _filedir
                            ;;
                    esac
                fi
                return
                ;;
            enable)
                if [[ $cur == -* ]]; then
                    COMPREPLY=($(compgen -W '--help --domain' -- "$cur"))
                else
                    case $prev in
                        --domain | -d)
                            _tbtools_complete_domains
                            ;;
                    esac
                fi
                return
                ;;
            *)
                if [[ $cur == -* ]]; then
                    COMPREPLY=($(compgen -W '--help' -- "$cur"))
                fi
                return
                ;;
        esac
    fi
} &&
    complete -F _tbtrace tbtrace

# ex: ts=4 sw=4 et filetype=sh

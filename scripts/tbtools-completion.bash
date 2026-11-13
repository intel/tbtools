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
# ln -s tbtools-completion.bash tbmonitor
# ln -s tbtools-completion.bash tbpd
# ln -s tbtools-completion.bash tbset
# ln -s tbtools-completion.bash tbstream
# ln -s tbtools-completion.bash tbtrace
# ln -s tbtools-completion.bash tbtunnels
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

# 1: "group" to list only Streams that have a ConfigFS directory present
_tbtools_complete_streams()
{
    local streams

    streams=$(tbstream list -S 2> /dev/null |
        sed 1d |
        awk -F, -v group="$1" '(group == "" || $3 == "true") && !seen[$1]++ { print $1 }' |
        xargs)
    COMPREPLY+=($(compgen -W "$streams" -- "$cur"))
}

# 1: Stream name
# 2...: stream devices already given on the command line
_tbtools_complete_stream_devices()
{
    local stream sdevs

    [[ ! $1 ]] && return
    stream=$1
    shift

    sdevs=$(tbstream list -S 2> /dev/null |
        sed 1d |
        awk -F, -v stream="$stream" -v used="$*" '
            BEGIN { n = split(used, u, " "); for (i = 1; i <= n; i++) skip[u[i]] }
            $1 == stream && $4 != "" && !($4 in skip) { print $4 }' |
        xargs)
    COMPREPLY+=($(compgen -W "$sdevs" -- "$cur"))
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
            --verbose --cap-id --vs-cap-id --nregs --help --drom' -- "$cur"))
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
            --cap-id | --vs-cap-id | --nregs | --drom | -[CVND])
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
            --binary --decimal --query --help --verbose --version' -- "$cur"))
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

_tbmonitor()
{
    local cur prev words cword domain
    _init_completion || return

    if [[ $cur == -* ]]; then
        COMPREPLY+=($(compgen -W '--domain --help --version' -- "$cur"))
    else
        case $prev in
            --domain | -d)
                _tbtools_complete_domains
                return
                ;;
        esac
    fi
} &&
    complete -F _tbmonitor tbmonitor

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

_tbstream()
{
    local cur prev words cword
    _init_completion || return

    local i cmd= stream= npos=0 all=
    local -a sdevs=()

    for ((i = 1; i < cword; i++)); do
        [[ ${words[i]} == --all ]] && all=1
        [[ ${words[i]} == -* ]] && continue
        if [[ ! $cmd ]]; then
            cmd=${words[i]}
        elif ((npos++ == 0)); then
            stream=${words[i]}
        else
            sdevs+=("${words[i]}")
        fi
    done

    if [[ $cur == -* ]]; then
        case $cmd in
            '')
                COMPREPLY=($(compgen -W '-h --help -V --version' -- "$cur"))
                ;;
            list)
                COMPREPLY=($(compgen -W '-S --script -h --help' -- "$cur"))
                ;;
            auto | add)
                COMPREPLY=($(compgen -W '-h --help' -- "$cur"))
                ;;
            remove)
                if [[ ! $all ]] && ((npos == 0)); then
                    COMPREPLY=($(compgen -W '--all -h --help' -- "$cur"))
                else
                    COMPREPLY=($(compgen -W '-h --help' -- "$cur"))
                fi
                ;;
        esac
        return
    fi

    case $cmd in
        '')
            COMPREPLY=($(compgen -W 'list auto add remove help' -- "$cur"))
            ;;
        auto | add)
            ((npos == 0)) && _tbtools_complete_streams
            ;;
        remove)
            [[ $all ]] && return
            if ((npos == 0)); then
                _tbtools_complete_streams group
            else
                _tbtools_complete_stream_devices "$stream" "${sdevs[@]}"
            fi
            ;;
        help)
            ((npos == 0)) &&
                COMPREPLY=($(compgen -W 'list auto add remove' -- "$cur"))
            ;;
    esac
} &&
    complete -F _tbstream tbstream

_tbtunnels()
{
    local cur prev words cword domain
    _init_completion || return

    if [[ $cur == -* ]]; then
        COMPREPLY+=($(compgen -W '--domain --verbose --help --version' -- "$cur"))
    else
        case $prev in
            --domain | -d)
                _tbtools_complete_domains
                return
                ;;
        esac
    fi
} &&
    complete -F _tbtunnels tbtunnels

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

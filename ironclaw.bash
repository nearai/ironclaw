_ironclaw() {
    local i cur prev opts cmd
    COMPREPLY=()
    if [[ "${BASH_VERSINFO[0]}" -ge 4 ]]; then
        cur="$2"
    else
        cur="${COMP_WORDS[COMP_CWORD]}"
    fi
    prev="$3"
    cmd=""
    opts=""

    for i in "${COMP_WORDS[@]:0:COMP_CWORD}"
    do
        case "${cmd},${i}" in
            ",$1")
                cmd="ironclaw"
                ;;
            ironclaw,channels)
                cmd="ironclaw__subcmd__channels"
                ;;
            ironclaw,completion)
                cmd="ironclaw__subcmd__completion"
                ;;
            ironclaw,config)
                cmd="ironclaw__subcmd__config"
                ;;
            ironclaw,doctor)
                cmd="ironclaw__subcmd__doctor"
                ;;
            ironclaw,extension)
                cmd="ironclaw__subcmd__extension"
                ;;
            ironclaw,help)
                cmd="ironclaw__subcmd__help"
                ;;
            ironclaw,hooks)
                cmd="ironclaw__subcmd__hooks"
                ;;
            ironclaw,logs)
                cmd="ironclaw__subcmd__logs"
                ;;
            ironclaw,models)
                cmd="ironclaw__subcmd__models"
                ;;
            ironclaw,onboard)
                cmd="ironclaw__subcmd__onboard"
                ;;
            ironclaw,profile)
                cmd="ironclaw__subcmd__profile"
                ;;
            ironclaw,repl)
                cmd="ironclaw__subcmd__repl"
                ;;
            ironclaw,run)
                cmd="ironclaw__subcmd__run"
                ;;
            ironclaw,serve)
                cmd="ironclaw__subcmd__serve"
                ;;
            ironclaw,service)
                cmd="ironclaw__subcmd__service"
                ;;
            ironclaw,skills)
                cmd="ironclaw__subcmd__skills"
                ;;
            ironclaw,status)
                cmd="ironclaw__subcmd__status"
                ;;
            ironclaw,traces)
                cmd="ironclaw__subcmd__traces"
                ;;
            ironclaw__subcmd__channels,help)
                cmd="ironclaw__subcmd__channels__subcmd__help"
                ;;
            ironclaw__subcmd__channels,list)
                cmd="ironclaw__subcmd__channels__subcmd__list"
                ;;
            ironclaw__subcmd__channels__subcmd__help,help)
                cmd="ironclaw__subcmd__channels__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__channels__subcmd__help,list)
                cmd="ironclaw__subcmd__channels__subcmd__help__subcmd__list"
                ;;
            ironclaw__subcmd__config,get)
                cmd="ironclaw__subcmd__config__subcmd__get"
                ;;
            ironclaw__subcmd__config,help)
                cmd="ironclaw__subcmd__config__subcmd__help"
                ;;
            ironclaw__subcmd__config,init)
                cmd="ironclaw__subcmd__config__subcmd__init"
                ;;
            ironclaw__subcmd__config,list)
                cmd="ironclaw__subcmd__config__subcmd__list"
                ;;
            ironclaw__subcmd__config,path)
                cmd="ironclaw__subcmd__config__subcmd__path"
                ;;
            ironclaw__subcmd__config__subcmd__help,get)
                cmd="ironclaw__subcmd__config__subcmd__help__subcmd__get"
                ;;
            ironclaw__subcmd__config__subcmd__help,help)
                cmd="ironclaw__subcmd__config__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__config__subcmd__help,init)
                cmd="ironclaw__subcmd__config__subcmd__help__subcmd__init"
                ;;
            ironclaw__subcmd__config__subcmd__help,list)
                cmd="ironclaw__subcmd__config__subcmd__help__subcmd__list"
                ;;
            ironclaw__subcmd__config__subcmd__help,path)
                cmd="ironclaw__subcmd__config__subcmd__help__subcmd__path"
                ;;
            ironclaw__subcmd__extension,activate)
                cmd="ironclaw__subcmd__extension__subcmd__activate"
                ;;
            ironclaw__subcmd__extension,help)
                cmd="ironclaw__subcmd__extension__subcmd__help"
                ;;
            ironclaw__subcmd__extension,install)
                cmd="ironclaw__subcmd__extension__subcmd__install"
                ;;
            ironclaw__subcmd__extension,remove)
                cmd="ironclaw__subcmd__extension__subcmd__remove"
                ;;
            ironclaw__subcmd__extension,search)
                cmd="ironclaw__subcmd__extension__subcmd__search"
                ;;
            ironclaw__subcmd__extension__subcmd__help,activate)
                cmd="ironclaw__subcmd__extension__subcmd__help__subcmd__activate"
                ;;
            ironclaw__subcmd__extension__subcmd__help,help)
                cmd="ironclaw__subcmd__extension__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__extension__subcmd__help,install)
                cmd="ironclaw__subcmd__extension__subcmd__help__subcmd__install"
                ;;
            ironclaw__subcmd__extension__subcmd__help,remove)
                cmd="ironclaw__subcmd__extension__subcmd__help__subcmd__remove"
                ;;
            ironclaw__subcmd__extension__subcmd__help,search)
                cmd="ironclaw__subcmd__extension__subcmd__help__subcmd__search"
                ;;
            ironclaw__subcmd__help,channels)
                cmd="ironclaw__subcmd__help__subcmd__channels"
                ;;
            ironclaw__subcmd__help,completion)
                cmd="ironclaw__subcmd__help__subcmd__completion"
                ;;
            ironclaw__subcmd__help,config)
                cmd="ironclaw__subcmd__help__subcmd__config"
                ;;
            ironclaw__subcmd__help,doctor)
                cmd="ironclaw__subcmd__help__subcmd__doctor"
                ;;
            ironclaw__subcmd__help,extension)
                cmd="ironclaw__subcmd__help__subcmd__extension"
                ;;
            ironclaw__subcmd__help,help)
                cmd="ironclaw__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__help,hooks)
                cmd="ironclaw__subcmd__help__subcmd__hooks"
                ;;
            ironclaw__subcmd__help,logs)
                cmd="ironclaw__subcmd__help__subcmd__logs"
                ;;
            ironclaw__subcmd__help,models)
                cmd="ironclaw__subcmd__help__subcmd__models"
                ;;
            ironclaw__subcmd__help,onboard)
                cmd="ironclaw__subcmd__help__subcmd__onboard"
                ;;
            ironclaw__subcmd__help,profile)
                cmd="ironclaw__subcmd__help__subcmd__profile"
                ;;
            ironclaw__subcmd__help,repl)
                cmd="ironclaw__subcmd__help__subcmd__repl"
                ;;
            ironclaw__subcmd__help,run)
                cmd="ironclaw__subcmd__help__subcmd__run"
                ;;
            ironclaw__subcmd__help,serve)
                cmd="ironclaw__subcmd__help__subcmd__serve"
                ;;
            ironclaw__subcmd__help,service)
                cmd="ironclaw__subcmd__help__subcmd__service"
                ;;
            ironclaw__subcmd__help,skills)
                cmd="ironclaw__subcmd__help__subcmd__skills"
                ;;
            ironclaw__subcmd__help,status)
                cmd="ironclaw__subcmd__help__subcmd__status"
                ;;
            ironclaw__subcmd__help,traces)
                cmd="ironclaw__subcmd__help__subcmd__traces"
                ;;
            ironclaw__subcmd__help__subcmd__channels,list)
                cmd="ironclaw__subcmd__help__subcmd__channels__subcmd__list"
                ;;
            ironclaw__subcmd__help__subcmd__config,get)
                cmd="ironclaw__subcmd__help__subcmd__config__subcmd__get"
                ;;
            ironclaw__subcmd__help__subcmd__config,init)
                cmd="ironclaw__subcmd__help__subcmd__config__subcmd__init"
                ;;
            ironclaw__subcmd__help__subcmd__config,list)
                cmd="ironclaw__subcmd__help__subcmd__config__subcmd__list"
                ;;
            ironclaw__subcmd__help__subcmd__config,path)
                cmd="ironclaw__subcmd__help__subcmd__config__subcmd__path"
                ;;
            ironclaw__subcmd__help__subcmd__extension,activate)
                cmd="ironclaw__subcmd__help__subcmd__extension__subcmd__activate"
                ;;
            ironclaw__subcmd__help__subcmd__extension,install)
                cmd="ironclaw__subcmd__help__subcmd__extension__subcmd__install"
                ;;
            ironclaw__subcmd__help__subcmd__extension,remove)
                cmd="ironclaw__subcmd__help__subcmd__extension__subcmd__remove"
                ;;
            ironclaw__subcmd__help__subcmd__extension,search)
                cmd="ironclaw__subcmd__help__subcmd__extension__subcmd__search"
                ;;
            ironclaw__subcmd__help__subcmd__hooks,list)
                cmd="ironclaw__subcmd__help__subcmd__hooks__subcmd__list"
                ;;
            ironclaw__subcmd__help__subcmd__models,list)
                cmd="ironclaw__subcmd__help__subcmd__models__subcmd__list"
                ;;
            ironclaw__subcmd__help__subcmd__models,set)
                cmd="ironclaw__subcmd__help__subcmd__models__subcmd__set"
                ;;
            ironclaw__subcmd__help__subcmd__models,set-provider)
                cmd="ironclaw__subcmd__help__subcmd__models__subcmd__set__subcmd__provider"
                ;;
            ironclaw__subcmd__help__subcmd__models,status)
                cmd="ironclaw__subcmd__help__subcmd__models__subcmd__status"
                ;;
            ironclaw__subcmd__help__subcmd__profile,list)
                cmd="ironclaw__subcmd__help__subcmd__profile__subcmd__list"
                ;;
            ironclaw__subcmd__help__subcmd__service,install)
                cmd="ironclaw__subcmd__help__subcmd__service__subcmd__install"
                ;;
            ironclaw__subcmd__help__subcmd__service,restart)
                cmd="ironclaw__subcmd__help__subcmd__service__subcmd__restart"
                ;;
            ironclaw__subcmd__help__subcmd__service,start)
                cmd="ironclaw__subcmd__help__subcmd__service__subcmd__start"
                ;;
            ironclaw__subcmd__help__subcmd__service,status)
                cmd="ironclaw__subcmd__help__subcmd__service__subcmd__status"
                ;;
            ironclaw__subcmd__help__subcmd__service,stop)
                cmd="ironclaw__subcmd__help__subcmd__service__subcmd__stop"
                ;;
            ironclaw__subcmd__help__subcmd__service,uninstall)
                cmd="ironclaw__subcmd__help__subcmd__service__subcmd__uninstall"
                ;;
            ironclaw__subcmd__help__subcmd__skills,list)
                cmd="ironclaw__subcmd__help__subcmd__skills__subcmd__list"
                ;;
            ironclaw__subcmd__help__subcmd__traces,credit)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__credit"
                ;;
            ironclaw__subcmd__help__subcmd__traces,enqueue)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__enqueue"
                ;;
            ironclaw__subcmd__help__subcmd__traces,enroll-instance)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__enroll__subcmd__instance"
                ;;
            ironclaw__subcmd__help__subcmd__traces,flush-queue)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__flush__subcmd__queue"
                ;;
            ironclaw__subcmd__help__subcmd__traces,ingest-health)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__ingest__subcmd__health"
                ;;
            ironclaw__subcmd__help__subcmd__traces,list-submissions)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__list__subcmd__submissions"
                ;;
            ironclaw__subcmd__help__subcmd__traces,opt-in)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__opt__subcmd__in"
                ;;
            ironclaw__subcmd__help__subcmd__traces,opt-out)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__opt__subcmd__out"
                ;;
            ironclaw__subcmd__help__subcmd__traces,preview)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__preview"
                ;;
            ironclaw__subcmd__help__subcmd__traces,profile)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__profile"
                ;;
            ironclaw__subcmd__help__subcmd__traces,queue-status)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__queue__subcmd__status"
                ;;
            ironclaw__subcmd__help__subcmd__traces,revoke)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__revoke"
                ;;
            ironclaw__subcmd__help__subcmd__traces,status)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__status"
                ;;
            ironclaw__subcmd__help__subcmd__traces,submit)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__submit"
                ;;
            ironclaw__subcmd__help__subcmd__traces__subcmd__profile,set)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__set"
                ;;
            ironclaw__subcmd__help__subcmd__traces__subcmd__profile,token)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__token"
                ;;
            ironclaw__subcmd__help__subcmd__traces__subcmd__profile,withdraw)
                cmd="ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__withdraw"
                ;;
            ironclaw__subcmd__hooks,help)
                cmd="ironclaw__subcmd__hooks__subcmd__help"
                ;;
            ironclaw__subcmd__hooks,list)
                cmd="ironclaw__subcmd__hooks__subcmd__list"
                ;;
            ironclaw__subcmd__hooks__subcmd__help,help)
                cmd="ironclaw__subcmd__hooks__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__hooks__subcmd__help,list)
                cmd="ironclaw__subcmd__hooks__subcmd__help__subcmd__list"
                ;;
            ironclaw__subcmd__models,help)
                cmd="ironclaw__subcmd__models__subcmd__help"
                ;;
            ironclaw__subcmd__models,list)
                cmd="ironclaw__subcmd__models__subcmd__list"
                ;;
            ironclaw__subcmd__models,set)
                cmd="ironclaw__subcmd__models__subcmd__set"
                ;;
            ironclaw__subcmd__models,set-provider)
                cmd="ironclaw__subcmd__models__subcmd__set__subcmd__provider"
                ;;
            ironclaw__subcmd__models,status)
                cmd="ironclaw__subcmd__models__subcmd__status"
                ;;
            ironclaw__subcmd__models__subcmd__help,help)
                cmd="ironclaw__subcmd__models__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__models__subcmd__help,list)
                cmd="ironclaw__subcmd__models__subcmd__help__subcmd__list"
                ;;
            ironclaw__subcmd__models__subcmd__help,set)
                cmd="ironclaw__subcmd__models__subcmd__help__subcmd__set"
                ;;
            ironclaw__subcmd__models__subcmd__help,set-provider)
                cmd="ironclaw__subcmd__models__subcmd__help__subcmd__set__subcmd__provider"
                ;;
            ironclaw__subcmd__models__subcmd__help,status)
                cmd="ironclaw__subcmd__models__subcmd__help__subcmd__status"
                ;;
            ironclaw__subcmd__profile,help)
                cmd="ironclaw__subcmd__profile__subcmd__help"
                ;;
            ironclaw__subcmd__profile,list)
                cmd="ironclaw__subcmd__profile__subcmd__list"
                ;;
            ironclaw__subcmd__profile__subcmd__help,help)
                cmd="ironclaw__subcmd__profile__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__profile__subcmd__help,list)
                cmd="ironclaw__subcmd__profile__subcmd__help__subcmd__list"
                ;;
            ironclaw__subcmd__service,help)
                cmd="ironclaw__subcmd__service__subcmd__help"
                ;;
            ironclaw__subcmd__service,install)
                cmd="ironclaw__subcmd__service__subcmd__install"
                ;;
            ironclaw__subcmd__service,restart)
                cmd="ironclaw__subcmd__service__subcmd__restart"
                ;;
            ironclaw__subcmd__service,start)
                cmd="ironclaw__subcmd__service__subcmd__start"
                ;;
            ironclaw__subcmd__service,status)
                cmd="ironclaw__subcmd__service__subcmd__status"
                ;;
            ironclaw__subcmd__service,stop)
                cmd="ironclaw__subcmd__service__subcmd__stop"
                ;;
            ironclaw__subcmd__service,uninstall)
                cmd="ironclaw__subcmd__service__subcmd__uninstall"
                ;;
            ironclaw__subcmd__service__subcmd__help,help)
                cmd="ironclaw__subcmd__service__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__service__subcmd__help,install)
                cmd="ironclaw__subcmd__service__subcmd__help__subcmd__install"
                ;;
            ironclaw__subcmd__service__subcmd__help,restart)
                cmd="ironclaw__subcmd__service__subcmd__help__subcmd__restart"
                ;;
            ironclaw__subcmd__service__subcmd__help,start)
                cmd="ironclaw__subcmd__service__subcmd__help__subcmd__start"
                ;;
            ironclaw__subcmd__service__subcmd__help,status)
                cmd="ironclaw__subcmd__service__subcmd__help__subcmd__status"
                ;;
            ironclaw__subcmd__service__subcmd__help,stop)
                cmd="ironclaw__subcmd__service__subcmd__help__subcmd__stop"
                ;;
            ironclaw__subcmd__service__subcmd__help,uninstall)
                cmd="ironclaw__subcmd__service__subcmd__help__subcmd__uninstall"
                ;;
            ironclaw__subcmd__skills,help)
                cmd="ironclaw__subcmd__skills__subcmd__help"
                ;;
            ironclaw__subcmd__skills,list)
                cmd="ironclaw__subcmd__skills__subcmd__list"
                ;;
            ironclaw__subcmd__skills__subcmd__help,help)
                cmd="ironclaw__subcmd__skills__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__skills__subcmd__help,list)
                cmd="ironclaw__subcmd__skills__subcmd__help__subcmd__list"
                ;;
            ironclaw__subcmd__traces,credit)
                cmd="ironclaw__subcmd__traces__subcmd__credit"
                ;;
            ironclaw__subcmd__traces,enqueue)
                cmd="ironclaw__subcmd__traces__subcmd__enqueue"
                ;;
            ironclaw__subcmd__traces,enroll-instance)
                cmd="ironclaw__subcmd__traces__subcmd__enroll__subcmd__instance"
                ;;
            ironclaw__subcmd__traces,flush-queue)
                cmd="ironclaw__subcmd__traces__subcmd__flush__subcmd__queue"
                ;;
            ironclaw__subcmd__traces,help)
                cmd="ironclaw__subcmd__traces__subcmd__help"
                ;;
            ironclaw__subcmd__traces,ingest-health)
                cmd="ironclaw__subcmd__traces__subcmd__ingest__subcmd__health"
                ;;
            ironclaw__subcmd__traces,list-submissions)
                cmd="ironclaw__subcmd__traces__subcmd__list__subcmd__submissions"
                ;;
            ironclaw__subcmd__traces,opt-in)
                cmd="ironclaw__subcmd__traces__subcmd__opt__subcmd__in"
                ;;
            ironclaw__subcmd__traces,opt-out)
                cmd="ironclaw__subcmd__traces__subcmd__opt__subcmd__out"
                ;;
            ironclaw__subcmd__traces,preview)
                cmd="ironclaw__subcmd__traces__subcmd__preview"
                ;;
            ironclaw__subcmd__traces,profile)
                cmd="ironclaw__subcmd__traces__subcmd__profile"
                ;;
            ironclaw__subcmd__traces,queue-status)
                cmd="ironclaw__subcmd__traces__subcmd__queue__subcmd__status"
                ;;
            ironclaw__subcmd__traces,revoke)
                cmd="ironclaw__subcmd__traces__subcmd__revoke"
                ;;
            ironclaw__subcmd__traces,status)
                cmd="ironclaw__subcmd__traces__subcmd__status"
                ;;
            ironclaw__subcmd__traces,submit)
                cmd="ironclaw__subcmd__traces__subcmd__submit"
                ;;
            ironclaw__subcmd__traces__subcmd__help,credit)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__credit"
                ;;
            ironclaw__subcmd__traces__subcmd__help,enqueue)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__enqueue"
                ;;
            ironclaw__subcmd__traces__subcmd__help,enroll-instance)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__enroll__subcmd__instance"
                ;;
            ironclaw__subcmd__traces__subcmd__help,flush-queue)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__flush__subcmd__queue"
                ;;
            ironclaw__subcmd__traces__subcmd__help,help)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__traces__subcmd__help,ingest-health)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__ingest__subcmd__health"
                ;;
            ironclaw__subcmd__traces__subcmd__help,list-submissions)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__list__subcmd__submissions"
                ;;
            ironclaw__subcmd__traces__subcmd__help,opt-in)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__opt__subcmd__in"
                ;;
            ironclaw__subcmd__traces__subcmd__help,opt-out)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__opt__subcmd__out"
                ;;
            ironclaw__subcmd__traces__subcmd__help,preview)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__preview"
                ;;
            ironclaw__subcmd__traces__subcmd__help,profile)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__profile"
                ;;
            ironclaw__subcmd__traces__subcmd__help,queue-status)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__queue__subcmd__status"
                ;;
            ironclaw__subcmd__traces__subcmd__help,revoke)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__revoke"
                ;;
            ironclaw__subcmd__traces__subcmd__help,status)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__status"
                ;;
            ironclaw__subcmd__traces__subcmd__help,submit)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__submit"
                ;;
            ironclaw__subcmd__traces__subcmd__help__subcmd__profile,set)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__set"
                ;;
            ironclaw__subcmd__traces__subcmd__help__subcmd__profile,token)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__token"
                ;;
            ironclaw__subcmd__traces__subcmd__help__subcmd__profile,withdraw)
                cmd="ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__withdraw"
                ;;
            ironclaw__subcmd__traces__subcmd__profile,help)
                cmd="ironclaw__subcmd__traces__subcmd__profile__subcmd__help"
                ;;
            ironclaw__subcmd__traces__subcmd__profile,set)
                cmd="ironclaw__subcmd__traces__subcmd__profile__subcmd__set"
                ;;
            ironclaw__subcmd__traces__subcmd__profile,token)
                cmd="ironclaw__subcmd__traces__subcmd__profile__subcmd__token"
                ;;
            ironclaw__subcmd__traces__subcmd__profile,withdraw)
                cmd="ironclaw__subcmd__traces__subcmd__profile__subcmd__withdraw"
                ;;
            ironclaw__subcmd__traces__subcmd__profile__subcmd__help,help)
                cmd="ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__help"
                ;;
            ironclaw__subcmd__traces__subcmd__profile__subcmd__help,set)
                cmd="ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__set"
                ;;
            ironclaw__subcmd__traces__subcmd__profile__subcmd__help,token)
                cmd="ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__token"
                ;;
            ironclaw__subcmd__traces__subcmd__profile__subcmd__help,withdraw)
                cmd="ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__withdraw"
                ;;
            *)
                ;;
        esac
    done

    case "${cmd}" in
        ironclaw)
            opts="-h -V --help --version channels completion config doctor extension hooks logs models onboard profile repl run serve service skills status traces help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 1 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__channels)
            opts="-h --help list help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__channels__subcmd__help)
            opts="list help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__channels__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__channels__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__channels__subcmd__list)
            opts="-v -h --verbose --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__completion)
            opts="-h --shell --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --shell)
                    COMPREPLY=($(compgen -W "bash elvish fish powershell zsh" -- "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config)
            opts="-h --help path init list get help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config__subcmd__get)
            opts="-h --json --help <KEY>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config__subcmd__help)
            opts="path init list get help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config__subcmd__help__subcmd__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config__subcmd__help__subcmd__init)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config__subcmd__help__subcmd__path)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config__subcmd__init)
            opts="-h --force --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config__subcmd__list)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__config__subcmd__path)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__doctor)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension)
            opts="-h --confirm-host-access --help search install activate remove help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension__subcmd__activate)
            opts="-h --json --confirm-host-access --help <ID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension__subcmd__help)
            opts="search install activate remove help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension__subcmd__help__subcmd__activate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension__subcmd__help__subcmd__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension__subcmd__help__subcmd__remove)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension__subcmd__help__subcmd__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension__subcmd__install)
            opts="-h --json --confirm-host-access --help <ID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension__subcmd__remove)
            opts="-h --json --confirm-host-access --help <ID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__extension__subcmd__search)
            opts="-h --json --confirm-host-access --help [QUERY]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help)
            opts="channels completion config doctor extension hooks logs models onboard profile repl run serve service skills status traces help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__channels)
            opts="list"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__channels__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__completion)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__config)
            opts="path init list get"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__config__subcmd__get)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__config__subcmd__init)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__config__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__config__subcmd__path)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__doctor)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__extension)
            opts="search install activate remove"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__extension__subcmd__activate)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__extension__subcmd__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__extension__subcmd__remove)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__extension__subcmd__search)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__hooks)
            opts="list"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__hooks__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__logs)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__models)
            opts="list status set set-provider"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__models__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__models__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__models__subcmd__set__subcmd__provider)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__models__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__onboard)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__profile)
            opts="list"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__profile__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__repl)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__run)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__serve)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__service)
            opts="install start stop restart status uninstall"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__service__subcmd__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__service__subcmd__restart)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__service__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__service__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__service__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__service__subcmd__uninstall)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__skills)
            opts="list"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__skills__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces)
            opts="opt-in opt-out enroll-instance status preview enqueue flush-queue queue-status credit submit list-submissions revoke ingest-health profile"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__credit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__enqueue)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__enroll__subcmd__instance)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__flush__subcmd__queue)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__ingest__subcmd__health)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__list__subcmd__submissions)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__opt__subcmd__in)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__opt__subcmd__out)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__preview)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__profile)
            opts="token set withdraw"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__token)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__profile__subcmd__withdraw)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__queue__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__revoke)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__help__subcmd__traces__subcmd__submit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__hooks)
            opts="-h --help list help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__hooks__subcmd__help)
            opts="list help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__hooks__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__hooks__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__hooks__subcmd__list)
            opts="-v -h --verbose --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__logs)
            opts="-v -h --verbose --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models)
            opts="-h --help list status set set-provider help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models__subcmd__help)
            opts="list status set set-provider help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models__subcmd__help__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models__subcmd__help__subcmd__set__subcmd__provider)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models__subcmd__list)
            opts="-v -h --verbose --json --help [PROVIDER]"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models__subcmd__set)
            opts="-h --help <MODEL>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models__subcmd__set__subcmd__provider)
            opts="-h --model --help <PROVIDER>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --model)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__models__subcmd__status)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__onboard)
            opts="-h --force --dry-run --import-history --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__profile)
            opts="-h --help list help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__profile__subcmd__help)
            opts="list help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__profile__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__profile__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__profile__subcmd__list)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__repl)
            opts="-h --confirm-host-access --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__run)
            opts="-m -h --message --dry-run --confirm-host-access --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --message)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -m)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__serve)
            opts="-h --host --port --confirm-host-access --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --host)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --port)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service)
            opts="-h --help install start stop restart status uninstall help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__help)
            opts="install start stop restart status uninstall help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__help__subcmd__install)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__help__subcmd__restart)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__help__subcmd__start)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__help__subcmd__stop)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__help__subcmd__uninstall)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__install)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__restart)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__start)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__status)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__stop)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__service__subcmd__uninstall)
            opts="-h --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__skills)
            opts="-h --help list help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__skills__subcmd__help)
            opts="list help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__skills__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__skills__subcmd__help__subcmd__list)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__skills__subcmd__list)
            opts="-v -h --verbose --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__status)
            opts="-h --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces)
            opts="-h --help opt-in opt-out enroll-instance status preview enqueue flush-queue queue-status credit submit list-submissions revoke ingest-health profile help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 2 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__credit)
            opts="-h --json --notice --notice-scope --ack --snooze-hours --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --notice-scope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --snooze-hours)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__enqueue)
            opts="-h --envelope --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --envelope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__enroll__subcmd__instance)
            opts="-h --invite --include-message-text --include-tool-payloads --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --invite)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__flush__subcmd__queue)
            opts="-h --limit --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --limit)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help)
            opts="opt-in opt-out enroll-instance status preview enqueue flush-queue queue-status credit submit list-submissions revoke ingest-health profile help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__credit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__enqueue)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__enroll__subcmd__instance)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__flush__subcmd__queue)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__ingest__subcmd__health)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__list__subcmd__submissions)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__opt__subcmd__in)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__opt__subcmd__out)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__preview)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__profile)
            opts="token set withdraw"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__token)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__profile__subcmd__withdraw)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__queue__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__revoke)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__status)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__help__subcmd__submit)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__ingest__subcmd__health)
            opts="-h --endpoint --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --endpoint)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__list__subcmd__submissions)
            opts="-h --json --summary --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__opt__subcmd__in)
            opts="-h --endpoint --user-scope --bearer-token-env --upload-token-issuer-url --upload-token-issuer-allowed-hosts --upload-token-audience --upload-token-tenant-id --upload-token-workload-token-env --upload-token-invite-code --upload-token-issuer-timeout-ms --include-message-text --include-tool-payloads --scope --selected-tools --allow-pii-review-bypass --min-submission-score --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --endpoint)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --user-scope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --bearer-token-env)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --upload-token-issuer-url)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --upload-token-issuer-allowed-hosts)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --upload-token-audience)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --upload-token-tenant-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --upload-token-workload-token-env)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --upload-token-invite-code)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --upload-token-issuer-timeout-ms)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --scope)
                    COMPREPLY=($(compgen -W "debugging-evaluation benchmark-only ranking-training model-training" -- "${cur}"))
                    return 0
                    ;;
                --selected-tools)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --min-submission-score)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__opt__subcmd__out)
            opts="-h --user-scope --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --user-scope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__preview)
            opts="-o -h --recorded-trace --include-message-text --include-tool-payloads --scope --channel --engine-version --contributor-id --credit-account-ref --output --enqueue --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --recorded-trace)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --scope)
                    COMPREPLY=($(compgen -W "debugging-evaluation benchmark-only ranking-training model-training" -- "${cur}"))
                    return 0
                    ;;
                --channel)
                    COMPREPLY=($(compgen -W "web cli telegram slack routine other" -- "${cur}"))
                    return 0
                    ;;
                --engine-version)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --contributor-id)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --credit-account-ref)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --output)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                -o)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__profile)
            opts="-h --help token set withdraw help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__profile__subcmd__help)
            opts="token set withdraw help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__help)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__set)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__token)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__profile__subcmd__help__subcmd__withdraw)
            opts=""
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 5 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__profile__subcmd__set)
            opts="-h --handle --bio --user-scope --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --handle)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --bio)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --user-scope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__profile__subcmd__token)
            opts="-h --user-scope --json --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --user-scope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__profile__subcmd__withdraw)
            opts="-h --user-scope --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 4 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --user-scope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__queue__subcmd__status)
            opts="-h --json --scope --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --scope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__revoke)
            opts="-h --endpoint --bearer-token-env --help <SUBMISSION_ID>"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --endpoint)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --bearer-token-env)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__status)
            opts="-h --json --user-scope --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --user-scope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
        ironclaw__subcmd__traces__subcmd__submit)
            opts="-h --envelope --endpoint --bearer-token-env --help"
            if [[ ${cur} == -* || ${COMP_CWORD} -eq 3 ]] ; then
                COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
                return 0
            fi
            case "${prev}" in
                --envelope)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --endpoint)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                --bearer-token-env)
                    COMPREPLY=($(compgen -f "${cur}"))
                    return 0
                    ;;
                *)
                    COMPREPLY=()
                    ;;
            esac
            COMPREPLY=( $(compgen -W "${opts}" -- "${cur}") )
            return 0
            ;;
    esac
}

if [[ "${BASH_VERSINFO[0]}" -eq 4 && "${BASH_VERSINFO[1]}" -ge 4 || "${BASH_VERSINFO[0]}" -gt 4 ]]; then
    complete -F _ironclaw -o nosort -o bashdefault -o default ironclaw
else
    complete -F _ironclaw -o bashdefault -o default ironclaw
fi

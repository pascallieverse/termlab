#!/bin/sh
# A stand-in for an agent CLI, used by the integration tests. Never calls a real agent.
#
# Prints a coloured banner, then reads commands line by line:
#   pwd           print the working directory as "PWD <path>"
#   size          print the terminal size as "SIZE <rows> <cols>"
#   env NAME      print "ENV NAME=<value>" (or "ENV NAME unset")
#   flood N       print N numbered lines
#   prompt        show a permission prompt, then print "ANSWER len=<n>" for the reply line
#   work N        print a line every 0.3 s, N times (an agent doing work)
#   exit N        exit with code N
#   anything else is echoed back as "echo: <line>"

printf '\033[32mFAKE AGENT READY\033[0m\n'
[ $# -gt 0 ] && printf 'ARGS: %s\n' "$*"

while IFS= read -r line; do
  case "$line" in
    pwd)
      printf 'PWD %s\n' "$(pwd -P)"
      ;;
    size)
      set -- $(stty size)
      printf 'SIZE %s %s\n' "$1" "$2"
      ;;
    "env "*)
      name=${line#env }
      if eval "[ -n \"\${$name+x}\" ]"; then
        eval "printf 'ENV %s=%s\n' \"$name\" \"\$$name\""
      else
        printf 'ENV %s unset\n' "$name"
      fi
      ;;
    "flood "*)
      n=${line#flood }
      i=1
      while [ "$i" -le "$n" ]; do
        printf 'flood line %06d\n' "$i"
        i=$((i + 1))
      done
      printf 'FLOOD DONE\n'
      ;;
    prompt)
      printf ' Bash command\n\n   npm test\n\n Do you want to proceed?\n'
      printf ' \342\235\257 1. Yes\n'
      printf "   2. Yes, and don't ask again for npm test commands\n"
      printf '   3. No, and tell me what to do differently (esc)\n'
      IFS= read -r answer
      # Like a full-screen TUI, redraw without the prompt once it's answered.
      printf '\033[2J\033[H'
      printf 'ANSWER len=%s\n' "${#answer}"
      ;;
    "work "*)
      n=${line#work }
      i=1
      while [ "$i" -le "$n" ]; do
        printf 'working step %s\n' "$i"
        sleep 0.3
        i=$((i + 1))
      done
      printf 'WORK DONE\n'
      ;;
    "exit "*)
      code=${line#exit }
      printf 'bye %s\n' "$code"
      exit "$code"
      ;;
    *)
      printf 'echo: %s\n' "$line"
      ;;
  esac
done
exit 0

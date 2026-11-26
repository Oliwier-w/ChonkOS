# shellcheck shell=bash
[ -n "${BASH_VERSION:-}" ] || return 0
case $- in *i*) ;; *) return 0 ;; esac

_chonkos_branch() {
    local branch
    branch=$(git symbolic-ref --quiet --short HEAD 2>/dev/null) && printf ' (%s)' "$branch"
}
PS1='\[\e[38;5;110m\]\u@\h \[\e[36m\]\w'
if command -v git >/dev/null 2>&1; then
    # Expand the branch when Bash draws the prompt.
    # shellcheck disable=SC2016
    PS1+='$(_chonkos_branch)'
fi
PS1+='\[\e[0m\]\$ '
if command -v fdfind >/dev/null 2>&1 && ! command -v fd >/dev/null 2>&1; then
    alias fd='fdfind'
fi
alias ll='ls -alF'
alias la='ls -A'
alias gs='git status -sb'
alias gl='git log --oneline -n 15'
if [ -r /usr/share/doc/fzf/examples/key-bindings.bash ]; then
    # shellcheck source=/dev/null
    . /usr/share/doc/fzf/examples/key-bindings.bash
fi
export HISTCONTROL=ignoreboth HISTSIZE=10000 HISTFILESIZE=20000
shopt -s histappend

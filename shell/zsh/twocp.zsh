autoload -Uz add-zsh-hook
zmodload zsh/datetime

typeset -g TWOCP_BIN="${TWOCP_BIN:-twocp}"
typeset -gi TWOCP_MAX_ROWS="${TWOCP_MAX_ROWS:-6}"
typeset -gi TWOCP_DEBOUNCE_MS="${TWOCP_DEBOUNCE_MS:-75}"
typeset -g __twocp_enabled=0
typeset -g __twocp_status='no_match'
typeset -g __twocp_provider_id=''
typeset -g __twocp_parser_status=''
typeset -gi __twocp_menu_visible=0
typeset -gi __twocp_selection_index=0
typeset -gi __twocp_replace_start=0
typeset -gi __twocp_replace_end=0
typeset -gi __twocp_truncated_count=0
typeset -gi __twocp_debounce_fd=-1
typeset -g __twocp_debounce_pid=''
typeset -ga __twocp_insert_texts=()
typeset -ga __twocp_displays=()
typeset -ga __twocp_annotations=()
typeset -ga __twocp_kinds=()

function __twocp_cancel_debounce() {
  if (( __twocp_debounce_fd >= 0 )); then
    zle -F "${__twocp_debounce_fd}"
    exec {__twocp_debounce_fd}<&-
    __twocp_debounce_fd=-1
  fi

  if [[ -n "${__twocp_debounce_pid}" ]]; then
    kill "${__twocp_debounce_pid}" 2>/dev/null
    wait "${__twocp_debounce_pid}" 2>/dev/null
    __twocp_debounce_pid=''
  fi
}

function __twocp_reset_state() {
  __twocp_status='no_match'
  __twocp_provider_id=''
  __twocp_parser_status=''
  __twocp_menu_visible=0
  __twocp_selection_index=0
  __twocp_replace_start=0
  __twocp_replace_end=0
  __twocp_truncated_count=0
  __twocp_insert_texts=()
  __twocp_displays=()
  __twocp_annotations=()
  __twocp_kinds=()
}

function __twocp_clear_menu() {
  __twocp_cancel_debounce
  __twocp_reset_state
  POSTDISPLAY=''
  zle -R
}

function __twocp_render_menu() {
  if (( ! __twocp_menu_visible )); then
    POSTDISPLAY=''
    zle -R
    return
  fi

  local limit=${#__twocp_displays[@]}
  if (( limit > TWOCP_MAX_ROWS )); then
    limit=$TWOCP_MAX_ROWS
  fi

  local -a lines=()
  local index display annotation prefix
  for (( index = 1; index <= limit; index += 1 )); do
    prefix='  '
    if (( index - 1 == __twocp_selection_index )); then
      prefix='> '
    fi

    display="${__twocp_displays[index]}"
    annotation="${__twocp_annotations[index]}"
    if [[ -n "${annotation}" ]]; then
      lines+=("${prefix}${display} | ${annotation}")
    else
      lines+=("${prefix}${display}")
    fi
  done

  if (( __twocp_truncated_count > 0 )); then
    lines+=("  +${__twocp_truncated_count} more")
  fi

  POSTDISPLAY=$'\n'"${(F)lines}"
  zle -R
}

function __twocp_refresh_now() {
  local response
  response="$("${TWOCP_BIN}" suggest \
    --shell zsh \
    --buffer "${BUFFER}" \
    --cursor "${CURSOR}" \
    --cursor-units chars \
    --cwd "${PWD}" \
    --columns "${COLUMNS:-0}" \
    --rows "${LINES:-0}" \
    --max-suggestions "${TWOCP_MAX_ROWS}" \
    --format zsh 2>/dev/null)" || {
      __twocp_clear_menu
      return
    }

  eval "${response}"

  if (( ${#__twocp_insert_texts[@]} == 0 )); then
    __twocp_menu_visible=0
    POSTDISPLAY=''
    zle -R
    return
  fi

  __twocp_menu_visible=1
  if (( __twocp_selection_index >= ${#__twocp_insert_texts[@]} )); then
    __twocp_selection_index=0
  fi
  __twocp_render_menu
}

function __twocp_debounce_ready() {
  local fd="$1"
  local discard=''

  zle -F "${fd}"
  read -r -u "${fd}" -k 1 discard 2>/dev/null || true
  exec {fd}<&-
  __twocp_debounce_fd=-1
  __twocp_debounce_pid=''
  __twocp_refresh_now
}

function __twocp_schedule_refresh() {
  local force="${1:-0}"
  __twocp_cancel_debounce

  if (( force )); then
    __twocp_refresh_now
    return
  fi

  exec {__twocp_debounce_fd}< <(
    sleep "$(( TWOCP_DEBOUNCE_MS ))e-3"
    print -r -- .
  )
  __twocp_debounce_pid=$!
  zle -F -w "${__twocp_debounce_fd}" __twocp-debounce-ready
}

function __twocp_apply_selection() {
  if (( ! __twocp_menu_visible )); then
    return 1
  fi

  local selection=$(( __twocp_selection_index + 1 ))
  local insert_text="${__twocp_insert_texts[selection]}"
  local prefix=''
  local suffix=''

  if (( __twocp_replace_start > 0 )); then
    prefix="${BUFFER[1,__twocp_replace_start]}"
  fi
  suffix="${BUFFER[__twocp_replace_end + 1,-1]}"
  BUFFER="${prefix}${insert_text}${suffix}"
  CURSOR=$(( __twocp_replace_start + ${#insert_text} ))

  __twocp_clear_menu
  zle redisplay
  return 0
}

function __twocp_self_insert() {
  zle .self-insert
  if [[ "${KEYS}" == ' ' ]]; then
    __twocp_schedule_refresh 1
  else
    __twocp_schedule_refresh 0
  fi
}

function __twocp_backward_delete_char() {
  zle .backward-delete-char
  __twocp_schedule_refresh 0
}

function __twocp_down_line_or_history() {
  if (( __twocp_menu_visible )); then
    if (( __twocp_selection_index + 1 < ${#__twocp_insert_texts[@]} )); then
      __twocp_selection_index=$(( __twocp_selection_index + 1 ))
      __twocp_render_menu
    fi
    return 0
  fi

  zle .down-line-or-history
}

function __twocp_up_line_or_history() {
  if (( __twocp_menu_visible )); then
    if (( __twocp_selection_index > 0 )); then
      __twocp_selection_index=$(( __twocp_selection_index - 1 ))
      __twocp_render_menu
    fi
    return 0
  fi

  zle .up-line-or-history
}

function __twocp_expand_or_complete() {
  if ! __twocp_apply_selection; then
    zle .expand-or-complete
  fi
}

function __twocp_dismiss_or_escape() {
  if (( __twocp_menu_visible )); then
    __twocp_clear_menu
    return 0
  fi

  zle .send-break
}

function __twocp_accept_line() {
  __twocp_clear_menu
  zle .accept-line
}

function twocp_native_complete() {
  __twocp_clear_menu
  zle .expand-or-complete
}

function __twocp_precmd_clear() {
  __twocp_reset_state
}

function twocp_zsh_enable() {
  if (( __twocp_enabled )); then
    return 0
  fi

  zle -N self-insert __twocp_self_insert
  zle -N backward-delete-char __twocp_backward_delete_char
  zle -N up-line-or-history __twocp_up_line_or_history
  zle -N down-line-or-history __twocp_down_line_or_history
  zle -N expand-or-complete __twocp_expand_or_complete
  zle -N twocp-dismiss-or-escape __twocp_dismiss_or_escape
  zle -N accept-line __twocp_accept_line
  zle -N twocp-native-complete twocp_native_complete
  zle -N __twocp-debounce-ready __twocp_debounce_ready

  bindkey '^[[A' up-line-or-history
  bindkey '^[[B' down-line-or-history
  bindkey '^I' expand-or-complete
  bindkey '\e' twocp-dismiss-or-escape
  bindkey '^X^I' twocp-native-complete

  add-zsh-hook precmd __twocp_precmd_clear
  add-zsh-hook preexec __twocp_precmd_clear

  __twocp_enabled=1
}

function twocp_zsh_disable() {
  if (( ! __twocp_enabled )); then
    return 0
  fi

  zle -A .self-insert self-insert
  zle -A .backward-delete-char backward-delete-char
  zle -A .up-line-or-history up-line-or-history
  zle -A .down-line-or-history down-line-or-history
  zle -A .expand-or-complete expand-or-complete
  zle -A .accept-line accept-line

  bindkey '^[[A' up-line-or-history
  bindkey '^[[B' down-line-or-history
  bindkey '^I' expand-or-complete

  add-zsh-hook -d precmd __twocp_precmd_clear 2>/dev/null
  add-zsh-hook -d preexec __twocp_precmd_clear 2>/dev/null

  __twocp_clear_menu
  __twocp_enabled=0
}

twocp_zsh_enable

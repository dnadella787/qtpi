autoload -Uz add-zsh-hook
zmodload zsh/terminfo 2>/dev/null || true

if (( ${+functions[twocp_zsh_disable]} )) && (( ${__twocp_enabled:-0} )); then
  twocp_zsh_disable 2>/dev/null || true
fi

typeset -g TWOCP_BIN="${TWOCP_BIN:-twocp}"
typeset -gi TWOCP_MAX_ROWS="${TWOCP_MAX_ROWS:-5}"
typeset -gi TWOCP_MAX_SUGGESTIONS="${TWOCP_MAX_SUGGESTIONS:-50}"
typeset -g TWOCP_KEY_SHOW="${TWOCP_KEY_SHOW:-^X2s}"
typeset -g TWOCP_KEY_ACCEPT="${TWOCP_KEY_ACCEPT:-^X2a}"
typeset -g TWOCP_KEY_DISMISS="${TWOCP_KEY_DISMISS:-^X2d}"
typeset -g TWOCP_KEY_NEXT="${TWOCP_KEY_NEXT:-^X2j}"
typeset -g TWOCP_KEY_PREVIOUS="${TWOCP_KEY_PREVIOUS:-^X2k}"
typeset -g TWOCP_KEY_ENTER="${TWOCP_KEY_ENTER:-^M}"
typeset -g TWOCP_KEY_ENTER_ALT="${TWOCP_KEY_ENTER_ALT:-^J}"
typeset -g TWOCP_KEY_DOWN="${TWOCP_KEY_DOWN:-^[[B}"
typeset -g TWOCP_KEY_UP="${TWOCP_KEY_UP:-^[[A}"
typeset -g TWOCP_KEY_DOWN_ALT="${TWOCP_KEY_DOWN_ALT:-^[OB}"
typeset -g TWOCP_KEY_UP_ALT="${TWOCP_KEY_UP_ALT:-^[OA}"
typeset -g TWOCP_AUTO_ROOTS="${TWOCP_AUTO_ROOTS:-git kubectl k}"
typeset -g TWOCP_DEBUG_LOG="${TWOCP_DEBUG_LOG:-}"

typeset -g __twocp_enabled=0
typeset -g __twocp_status='no_match'
typeset -g __twocp_provider_id=''
typeset -g __twocp_parser_status=''
typeset -g __twocp_request_buffer=''
typeset -g __twocp_dynamic_slot_id=''
typeset -g __twocp_lookup_status='not_checked'
typeset -g __twocp_cache_status='not_checked'
typeset -g __twocp_saved_postdisplay=''
typeset -g __twocp_rendered_postdisplay=''
typeset -gi __twocp_owns_postdisplay=0
typeset -gi __twocp_autosuggest_suppressed=0
typeset -gi __twocp_menu_visible=0
typeset -gi __twocp_selection_index=0
typeset -gi __twocp_replace_start=0
typeset -gi __twocp_replace_end=0
typeset -gi __twocp_request_cursor=0
typeset -gi __twocp_truncated_count=0
typeset -gi __twocp_lookup_count=0
typeset -gi __twocp_lookup_time_ms=0
typeset -gi __twocp_after_widget_fd=-1
typeset -g __twocp_after_widget_pid=''
typeset -g __twocp_after_widget_action=''
typeset -ga __twocp_insert_texts=()
typeset -ga __twocp_displays=()
typeset -ga __twocp_annotations=()
typeset -ga __twocp_kinds=()
typeset -gA __twocp_saved_bindings=()
typeset -gA __twocp_managed_bindings=()
typeset -gA __twocp_saved_widget_aliases=()
typeset -gA __twocp_saved_widgets_by_widget=()
typeset -gA __twocp_saved_widget_aliases_by_widget=()
typeset -g __twocp_self_insert_saved=0
typeset -g __twocp_self_insert_alias='__twocp-orig-self-insert'

function __twocp_reset_state() {
  __twocp_status='no_match'
  __twocp_provider_id=''
  __twocp_parser_status=''
  __twocp_request_buffer=''
  __twocp_dynamic_slot_id=''
  __twocp_lookup_status='not_checked'
  __twocp_cache_status='not_checked'
  __twocp_menu_visible=0
  __twocp_selection_index=0
  __twocp_replace_start=0
  __twocp_replace_end=0
  __twocp_request_cursor=0
  __twocp_truncated_count=0
  __twocp_lookup_count=0
  __twocp_lookup_time_ms=0
  __twocp_insert_texts=()
  __twocp_displays=()
  __twocp_annotations=()
  __twocp_kinds=()
}

function __twocp_debug() {
  if [[ -n "${TWOCP_DEBUG_LOG}" ]]; then
    print -r -- "$*" >>| "${TWOCP_DEBUG_LOG}" 2>/dev/null || true
  fi
}

function __twocp_cancel_after_widget_refresh() {
  if (( __twocp_after_widget_fd >= 0 )); then
    zle -F "${__twocp_after_widget_fd}" 2>/dev/null || true
    exec {__twocp_after_widget_fd}<&- 2>/dev/null || true
    __twocp_after_widget_fd=-1
  fi

  if [[ -n "${__twocp_after_widget_pid}" ]]; then
    kill "${__twocp_after_widget_pid}" 2>/dev/null || true
    wait "${__twocp_after_widget_pid}" 2>/dev/null || true
    __twocp_after_widget_pid=''
  fi
  __twocp_after_widget_action=''
}

function __twocp_redraw() {
  if [[ -n "${WIDGET:-}" ]]; then
    zle -I 2>/dev/null || true
    zle -R 2>/dev/null || true
    zle redisplay 2>/dev/null || true
  fi
}

function __twocp_redisplay() {
  if [[ -n "${WIDGET:-}" ]]; then
    zle redisplay 2>/dev/null || true
  fi
}

function __twocp_set_status_message() {
  if [[ -n "${WIDGET:-}" ]]; then
    zle -M "$1" 2>/dev/null || true
  fi
}

function __twocp_take_postdisplay() {
  if (( ! __twocp_owns_postdisplay )); then
    __twocp_saved_postdisplay="${POSTDISPLAY-}"
    __twocp_owns_postdisplay=1
  fi
}

function __twocp_clear_highlight() {
  :
}

function __twocp_disable_autosuggestions() {
  if (( __twocp_autosuggest_suppressed )); then
    return 0
  fi

  if (( ${+functions[_zsh_autosuggest_disable]} )); then
    _zsh_autosuggest_disable
    __twocp_autosuggest_suppressed=1
  elif (( ${+functions[_zsh_autosuggest_clear]} )); then
    _zsh_autosuggest_clear
  fi
}

function __twocp_enable_autosuggestions() {
  if (( ! __twocp_autosuggest_suppressed )); then
    return 0
  fi

  if (( ${+functions[_zsh_autosuggest_enable]} )); then
    _zsh_autosuggest_enable
  fi
  __twocp_autosuggest_suppressed=0
}

function __twocp_restore_postdisplay() {
  __twocp_clear_highlight
  if (( __twocp_owns_postdisplay )); then
    if [[ "${POSTDISPLAY-}" == "${__twocp_rendered_postdisplay}" ]]; then
      POSTDISPLAY="${__twocp_saved_postdisplay}"
    fi
    __twocp_saved_postdisplay=''
    __twocp_rendered_postdisplay=''
    __twocp_owns_postdisplay=0
  fi
  __twocp_set_status_message ''
  __twocp_enable_autosuggestions
  __twocp_redraw
}

function __twocp_invalidate_menu() {
  __twocp_cancel_after_widget_refresh
  __twocp_reset_state
  __twocp_restore_postdisplay
}

function __twocp_render_menu() {
  if (( ! __twocp_menu_visible )); then
    __twocp_restore_postdisplay
    return
  fi

  local total=${#__twocp_displays[@]}
  local window_start=0
  if (( __twocp_selection_index >= TWOCP_MAX_ROWS )); then
    window_start=$(( __twocp_selection_index - TWOCP_MAX_ROWS + 1 ))
  fi

  local window_end=$(( window_start + TWOCP_MAX_ROWS ))
  if (( window_end > total )); then
    window_end=$total
  fi

  local display_width=0
  local annotation_budget=0
  local display_len
  local available_columns=${COLUMNS:-80}
  local content_budget=$(( available_columns - 6 ))
  if (( content_budget < 24 )); then
    content_budget=24
  fi

  for (( index = window_start + 1; index <= window_end; index += 1 )); do
    display_len=${#__twocp_displays[index]}
    if (( display_len > display_width )); then
      display_width=$display_len
    fi
  done

  if (( display_width > content_budget / 2 )); then
    display_width=$(( content_budget / 2 ))
  fi
  if (( display_width < 8 )); then
    display_width=8
  fi
  annotation_budget=$(( content_budget - display_width - 3 ))
  if (( annotation_budget < 0 )); then
    annotation_budget=0
  fi

  local -a lines=()
  local index display annotation padded_display row selected prefix
  for (( index = window_start + 1; index <= window_end; index += 1 )); do
    selected=0
    if (( index - 1 == __twocp_selection_index )); then
      selected=1
    fi

    display="${__twocp_displays[index]}"
    annotation="${__twocp_annotations[index]}"
    if (( ${#display} > display_width )); then
      if (( display_width > 3 )); then
        display="${display[1,$(( display_width - 3 ))]}..."
      fi
    fi

    padded_display="${(r:display_width:: :)display}"
    prefix='[ ] '
    if (( selected )); then
      prefix='[>] '
    fi
    suffix=''
    if [[ -n "${annotation}" ]] && (( annotation_budget > 0 )); then
      if (( ${#annotation} > annotation_budget )); then
        if (( annotation_budget > 3 )); then
          annotation="${annotation[1,$(( annotation_budget - 3 ))]}..."
        else
          annotation=''
        fi
      fi
      if [[ -n "${annotation}" ]]; then
        row="${prefix}${padded_display} | ${annotation}"
      else
        row="${prefix}${padded_display}"
      fi
    else
      row="${prefix}${padded_display}"
    fi

    lines+=("${row}")
  done

  __twocp_disable_autosuggestions
  __twocp_take_postdisplay
  __twocp_rendered_postdisplay=$'\n'"${(F)lines}"
  POSTDISPLAY="${__twocp_rendered_postdisplay}"
  if (( total > 0 )); then
    local selected_display="${__twocp_displays[$(( __twocp_selection_index + 1 ))]}"
    __twocp_set_status_message "2cp [$(( __twocp_selection_index + 1 ))/${total}]: ${selected_display}"
  else
    __twocp_set_status_message ''
  fi
  __twocp_clear_highlight
  __twocp_redraw
}

function __twocp_refresh_now() {
  __twocp_cancel_after_widget_refresh

  local response
  __twocp_debug "refresh:start buffer=${(qqq)BUFFER} cursor=${CURSOR} bin=${TWOCP_BIN}"
  response="$("${TWOCP_BIN}" suggest \
    --shell zsh \
    --buffer "${BUFFER}" \
    --cursor "${CURSOR}" \
    --cursor-units chars \
    --cwd "${PWD}" \
    --columns "${COLUMNS:-0}" \
    --rows "${LINES:-0}" \
    --max-suggestions "${TWOCP_MAX_SUGGESTIONS}" \
    --format zsh 2>/dev/null)" || {
      __twocp_debug "refresh:error buffer=${(qqq)BUFFER} cursor=${CURSOR} bin=${TWOCP_BIN}"
      __twocp_invalidate_menu
      return
    }

  eval "${response}"
  __twocp_debug "refresh:done status=${__twocp_status} count=${#__twocp_insert_texts[@]} request=${(qqq)__twocp_request_buffer} cursor=${__twocp_request_cursor}"

  if (( ${#__twocp_insert_texts[@]} == 0 )); then
    __twocp_menu_visible=0
    __twocp_restore_postdisplay
    return
  fi

  __twocp_menu_visible=1
  if (( __twocp_selection_index >= ${#__twocp_insert_texts[@]} )); then
    __twocp_selection_index=0
  fi
  __twocp_render_menu
}

function __twocp_after_widget_ready() {
  local fd="$1"
  local discard=''

  __twocp_debug "after-widget:ready action=${__twocp_after_widget_action:-refresh} buffer=${(qqq)BUFFER} cursor=${CURSOR}"
  zle -F "${fd}" 2>/dev/null || true
  read -r -u "${fd}" -k 1 discard 2>/dev/null || true
  exec {fd}<&- 2>/dev/null || true
  __twocp_after_widget_fd=-1
  __twocp_after_widget_pid=''
  local action="${__twocp_after_widget_action:-refresh}"
  __twocp_after_widget_action=''
  if [[ "${action}" == 'render' ]]; then
    __twocp_render_menu
  else
    __twocp_refresh_now
  fi
}

function __twocp_schedule_after_widget_refresh() {
  __twocp_debug "after-widget:schedule-refresh buffer=${(qqq)BUFFER} cursor=${CURSOR}"
  __twocp_cancel_after_widget_refresh
  __twocp_after_widget_action='refresh'
  exec {__twocp_after_widget_fd}< <(
    sleep 0.01
    print -r -- .
  )
  __twocp_after_widget_pid=$!
  zle -F -w "${__twocp_after_widget_fd}" __twocp-after-widget-ready
}

function __twocp_schedule_after_widget_render() {
  __twocp_debug "after-widget:schedule-render buffer=${(qqq)BUFFER} cursor=${CURSOR}"
  __twocp_cancel_after_widget_refresh
  __twocp_after_widget_action='render'
  exec {__twocp_after_widget_fd}< <(
    sleep 0.01
    print -r -- .
  )
  __twocp_after_widget_pid=$!
  zle -F -w "${__twocp_after_widget_fd}" __twocp-after-widget-ready
}

function __twocp_menu_current() {
  if (( ! __twocp_menu_visible )); then
    return 1
  fi

  if [[ "${BUFFER}" != "${__twocp_request_buffer}" ]] || (( CURSOR != __twocp_request_cursor )); then
    __twocp_invalidate_menu
    return 1
  fi

  return 0
}

function __twocp_apply_selection() {
  if ! __twocp_menu_current; then
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

  __twocp_invalidate_menu
  if __twocp_should_auto_refresh_after_space; then
    __twocp_schedule_after_widget_refresh
  fi
  __twocp_redisplay
  return 0
}

function __twocp_enter_should_accept_selection() {
  if ! __twocp_menu_current; then
    return 1
  fi

  if (( ${#__twocp_insert_texts[@]} == 0 )); then
    __twocp_invalidate_menu
    return 1
  fi

  local selection=$(( __twocp_selection_index + 1 ))
  local insert_text="${__twocp_insert_texts[selection]}"
  local current_fragment=''
  local insert_without_space="${insert_text% }"

  if (( __twocp_replace_end > __twocp_replace_start )); then
    current_fragment="${BUFFER[$(( __twocp_replace_start + 1 )),__twocp_replace_end]}"
  fi

  if [[ -z "${current_fragment}" ]]; then
    return 0
  fi

  if [[ "${insert_without_space}" == "${current_fragment}" ]]; then
    return 1
  fi

  return 0
}

function twocp_show_or_refresh() {
  __twocp_schedule_after_widget_refresh
}

function twocp_accept_suggestion() {
  __twocp_apply_selection || true
}

function twocp_accept_or_original() {
  __twocp_debug "enter widget=${WIDGET:-} keys=${(qqq)KEYS} keymap=${KEYMAP:-} visible=${__twocp_menu_visible} buffer=${(qqq)BUFFER}"
  if __twocp_enter_should_accept_selection; then
    __twocp_apply_selection || true
    return 0
  fi

  __twocp_call_saved_widget "${KEYS}"
}

function twocp_dismiss_suggestions() {
  __twocp_invalidate_menu
}

function twocp_next_suggestion() {
  if ! __twocp_menu_current; then
    return 0
  fi

  __twocp_debug "menu:next widget=${WIDGET:-} keys=${(qqq)KEYS} selection=${__twocp_selection_index} count=${#__twocp_insert_texts[@]}"
  __twocp_cancel_after_widget_refresh
  if (( __twocp_selection_index + 1 < ${#__twocp_insert_texts[@]} )); then
    __twocp_selection_index=$(( __twocp_selection_index + 1 ))
    __twocp_schedule_after_widget_render
  fi
}

function twocp_previous_suggestion() {
  if ! __twocp_menu_current; then
    return 0
  fi

  __twocp_debug "menu:previous widget=${WIDGET:-} keys=${(qqq)KEYS} selection=${__twocp_selection_index} count=${#__twocp_insert_texts[@]}"
  __twocp_cancel_after_widget_refresh
  if (( __twocp_selection_index > 0 )); then
    __twocp_selection_index=$(( __twocp_selection_index - 1 ))
    __twocp_schedule_after_widget_render
  fi
}

function __twocp_call_saved_widget() {
  local key="$1"
  local keymap="${KEYMAP:-main}"
  local managed_widget="${WIDGET:-}"
  local binding_id="$(__twocp_binding_id "${keymap}" "${key}")"
  local widget_binding_id="$(__twocp_binding_id "${keymap}" "${managed_widget}")"
  local saved="${__twocp_saved_bindings[$binding_id]-}"
  local saved_alias="${__twocp_saved_widget_aliases[$binding_id]-}"
  local widget_saved="${__twocp_saved_widgets_by_widget[$widget_binding_id]-}"
  local widget_saved_alias="${__twocp_saved_widget_aliases_by_widget[$widget_binding_id]-}"
  local use_alias=1

  __twocp_debug "saved-widget:start widget=${managed_widget} keymap=${keymap} key=${(qqq)key} saved=${saved} alias=${saved_alias} widget_saved=${widget_saved} widget_alias=${widget_saved_alias} buffer=${(qqq)BUFFER}"
  __twocp_cancel_after_widget_refresh
  __twocp_invalidate_menu
  if [[ "${managed_widget}" == 'twocp-up-or-original' || "${managed_widget}" == 'twocp-down-or-original' ]]; then
    use_alias=0
  fi
  if (( use_alias )) && [[ -n "${widget_saved_alias}" ]]; then
    __twocp_debug "saved-widget:widget-alias widget=${widget_saved_alias}"
    zle "${widget_saved_alias}" 2>/dev/null && return 0
  fi
  if (( use_alias )) && [[ -n "${saved_alias}" ]]; then
    __twocp_debug "saved-widget:alias widget=${saved_alias}"
    zle "${saved_alias}" 2>/dev/null && return 0
  fi
  saved="${widget_saved:-$saved}"
  saved="$(__twocp_resolve_original_widget "${keymap}" "${managed_widget}" "${saved}")"
  saved="$(__twocp_normalize_fallback_widget "${managed_widget}" "${saved}")"
  __twocp_debug "saved-widget:resolved widget=${saved}"

  if [[ -z "${saved}" || "${saved}" == 'undefined-key' ]]; then
    zle .undefined-key 2>/dev/null || true
    return 0
  fi

  zle "${saved}"
}

function __twocp_call_saved_self_insert() {
  if (( __twocp_self_insert_saved )); then
    zle "${__twocp_self_insert_alias}" 2>/dev/null && return 0
  fi

  zle .self-insert
}

function __twocp_normalize_fallback_widget() {
  local managed_widget="$1"
  local saved_widget="$2"

  case "${managed_widget}" in
    twocp-up-or-original)
      case "${saved_widget}" in
        up-line-or-beginning-search|history-substring-search-up)
          print -r -- up-line-or-history
          return 0
          ;;
      esac
      ;;
    twocp-down-or-original)
      case "${saved_widget}" in
        down-line-or-beginning-search|history-substring-search-down)
          print -r -- down-line-or-history
          return 0
          ;;
      esac
      ;;
  esac

  print -r -- "${saved_widget}"
}

function twocp_down_or_original() {
  if __twocp_menu_current; then
    twocp_next_suggestion
    return 0
  fi

  __twocp_call_saved_widget "${KEYS}"
}

function twocp_up_or_original() {
  if __twocp_menu_current; then
    twocp_previous_suggestion
    return 0
  fi

  __twocp_call_saved_widget "${KEYS}"
}

function __twocp_supported_auto_root() {
  local command="$1"
  local root
  for root in ${(z)TWOCP_AUTO_ROOTS}; do
    if [[ "${command}" == "${root}" ]]; then
      return 0
    fi
  done

  return 1
}

function __twocp_should_auto_refresh_after_space() {
  if [[ "${BUFFER}" != *' ' ]]; then
    return 1
  fi

  local before_space="${BUFFER[1,-2]}"
  if [[ -z "${before_space//[[:space:]]/}" ]]; then
    return 1
  fi

  local -a tokens
  tokens=(${(z)before_space})
  if (( ${#tokens[@]} == 0 )); then
    return 1
  fi

  __twocp_supported_auto_root "${tokens[1]}"
}

function twocp_self_insert() {
  __twocp_call_saved_self_insert
  __twocp_debug "self-insert keys=${(qqq)KEYS} buffer=${(qqq)BUFFER} cursor=${CURSOR} visible=${__twocp_menu_visible}"
  if __twocp_should_auto_refresh_after_space; then
    __twocp_debug "self-insert:auto-root"
    __twocp_schedule_after_widget_refresh
  elif (( __twocp_menu_visible )); then
    __twocp_debug "self-insert:refresh-visible"
    __twocp_schedule_after_widget_refresh
  fi
}

function twocp_space_maybe_show() {
  __twocp_debug "space:start keys=${(qqq)KEYS} keymap=${KEYMAP:-} buffer=${(qqq)BUFFER} cursor=${CURSOR}"
  __twocp_call_saved_widget "${KEYS}"
  __twocp_debug "space buffer=${(qqq)BUFFER} cursor=${CURSOR} visible=${__twocp_menu_visible}"
  if __twocp_should_auto_refresh_after_space; then
    __twocp_debug "space:auto-root"
    __twocp_schedule_after_widget_refresh
  elif (( __twocp_menu_visible )); then
    __twocp_debug "space:refresh-visible"
    __twocp_schedule_after_widget_refresh
  fi
}

function __twocp_precmd_clear() {
  __twocp_cancel_after_widget_refresh
  __twocp_reset_state
  __twocp_restore_postdisplay
}

function __twocp_widget_for_key() {
  local key="$1"
  local keymap="${2:-}"
  local binding

  if [[ -n "${keymap}" ]]; then
    binding="$(bindkey -M "${keymap}" "${key}" 2>/dev/null)" || {
      print -r -- 'undefined-key'
      return 0
    }
  else
    binding="$(bindkey "${key}" 2>/dev/null)" || {
      print -r -- 'undefined-key'
      return 0
    }
  fi

  if [[ "${binding}" == *' undefined-key' ]]; then
    print -r -- 'undefined-key'
    return 0
  fi

  print -r -- "${binding##* }"
}

function __twocp_keymap_exists() {
  bindkey -M "$1" >/dev/null 2>&1
}

function __twocp_binding_id() {
  print -r -- "$1:$2"
}

function __twocp_is_managed_widget() {
  local widget="$1"
  [[ -n "${widget}" && ( "${widget}" == twocp-* || "${widget}" == __twocp-after-widget-ready ) ]]
}

function __twocp_saved_widget_for_managed_widget() {
  local keymap="$1"
  local widget="$2"
  local binding_id saved

  for binding_id in ${(k)__twocp_managed_bindings}; do
    if [[ "${binding_id}" != "${keymap}:"* ]]; then
      continue
    fi

    if [[ "${__twocp_managed_bindings[$binding_id]}" != "${widget}" ]]; then
      continue
    fi

    saved="${__twocp_saved_bindings[$binding_id]-}"
    if [[ -n "${saved}" && "${saved}" != 'undefined-key' ]] && ! __twocp_is_managed_widget "${saved}"; then
      print -r -- "${saved}"
      return 0
    fi
  done

  return 1
}

function __twocp_default_widget_for_managed_widget() {
  case "$1" in
    twocp-accept-or-original)
      print -r -- accept-line
      ;;
    twocp-down-or-original)
      print -r -- down-line-or-history
      ;;
    twocp-up-or-original)
      print -r -- up-line-or-history
      ;;
    twocp-space-maybe-show)
      print -r -- self-insert
      ;;
    *)
      return 1
      ;;
  esac
}

function __twocp_resolve_original_widget() {
  local keymap="$1"
  local widget="$2"
  local resolved="$3"

  if __twocp_is_managed_widget "${resolved}"; then
    resolved="$(__twocp_saved_widget_for_managed_widget "${keymap}" "${resolved}")"
  fi

  if [[ -z "${resolved}" || "${resolved}" == 'undefined-key' ]] && __twocp_is_managed_widget "${widget}"; then
    resolved="$(__twocp_default_widget_for_managed_widget "${widget}")"
  fi

  if [[ -n "${resolved}" ]]; then
    print -r -- "${resolved}"
    return 0
  fi

  return 1
}

function __twocp_bind_widget_in_keymap() {
  local keymap="$1"
  local key="$2"
  local widget="$3"
  local binding_id="$(__twocp_binding_id "${keymap}" "${key}")"
  local widget_binding_id="$(__twocp_binding_id "${keymap}" "${widget}")"
  local saved_widget=''
  local saved_alias=''

  if [[ -z "${__twocp_saved_bindings[$binding_id]+set}" ]]; then
    saved_widget="$(__twocp_resolve_original_widget "${keymap}" "${widget}" "$(__twocp_widget_for_key "${key}" "${keymap}")")"
    __twocp_saved_bindings[$binding_id]="${saved_widget}"
    if [[ -n "${saved_widget}" && "${saved_widget}" != 'undefined-key' ]]; then
      saved_alias="__twocp-saved-widget-$(( ${#__twocp_saved_widget_aliases[@]} + 1 ))"
      if zle -A "${saved_widget}" "${saved_alias}" 2>/dev/null; then
        __twocp_saved_widget_aliases[$binding_id]="${saved_alias}"
      else
        __twocp_saved_widget_aliases[$binding_id]=''
      fi
    fi
  fi

  if [[ -z "${__twocp_saved_widgets_by_widget[$widget_binding_id]+set}" ]]; then
    __twocp_saved_widgets_by_widget[$widget_binding_id]="${__twocp_saved_bindings[$binding_id]-}"
  fi

  if [[ -z "${__twocp_saved_widget_aliases_by_widget[$widget_binding_id]+set}" ]]; then
    __twocp_saved_widget_aliases_by_widget[$widget_binding_id]="${__twocp_saved_widget_aliases[$binding_id]-}"
  fi

  __twocp_managed_bindings[$binding_id]="${widget}"
  bindkey -M "${keymap}" "${key}" "${widget}"
}

function __twocp_restore_binding_in_keymap() {
  local keymap="$1"
  local key="$2"
  local binding_id="$(__twocp_binding_id "${keymap}" "${key}")"

  if [[ -z "${__twocp_managed_bindings[$binding_id]+set}" ]]; then
    return 0
  fi

  local widget="${__twocp_managed_bindings[$binding_id]}"
  local widget_binding_id="$(__twocp_binding_id "${keymap}" "${widget}")"
  local saved="${__twocp_saved_bindings[$binding_id]-}"
  local current="$(__twocp_widget_for_key "${key}" "${keymap}")"

  saved="$(__twocp_resolve_original_widget "${keymap}" "${widget}" "${saved}")"

  if [[ "${current}" == "${widget}" ]]; then
    if [[ -z "${saved}" || "${saved}" == 'undefined-key' ]]; then
      bindkey -M "${keymap}" -r "${key}" 2>/dev/null
    else
      bindkey -M "${keymap}" "${key}" "${saved}"
    fi
  fi

  unset "__twocp_saved_bindings[$binding_id]"
  unset "__twocp_managed_bindings[$binding_id]"
  unset "__twocp_saved_widget_aliases[$binding_id]"
  unset "__twocp_saved_widgets_by_widget[$widget_binding_id]"
  unset "__twocp_saved_widget_aliases_by_widget[$widget_binding_id]"
}

function __twocp_bind_widget() {
  local key="$1"
  local widget="$2"

  if [[ -z "${key}" ]]; then
    return 0
  fi

  local keymap
  for keymap in main emacs viins vicmd; do
    if __twocp_keymap_exists "${keymap}"; then
      __twocp_bind_widget_in_keymap "${keymap}" "${key}" "${widget}"
    fi
  done
}

function __twocp_restore_binding() {
  local key="$1"

  if [[ -z "${key}" ]]; then
    return 0
  fi

  local keymap
  for keymap in main emacs viins vicmd; do
    if __twocp_keymap_exists "${keymap}"; then
      __twocp_restore_binding_in_keymap "${keymap}" "${key}"
    fi
  done
}

function twocp_zsh_enable() {
  if (( __twocp_enabled )); then
    return 0
  fi

  zle -N twocp-show-or-refresh twocp_show_or_refresh
  zle -N twocp-accept-suggestion twocp_accept_suggestion
  zle -N twocp-accept-or-original twocp_accept_or_original
  zle -N twocp-dismiss-suggestions twocp_dismiss_suggestions
  zle -N twocp-next-suggestion twocp_next_suggestion
  zle -N twocp-previous-suggestion twocp_previous_suggestion
  zle -N twocp-down-or-original twocp_down_or_original
  zle -N twocp-up-or-original twocp_up_or_original
  zle -N twocp-space-maybe-show twocp_space_maybe_show
  zle -N __twocp-after-widget-ready __twocp_after_widget_ready
  zle -A self-insert "${__twocp_self_insert_alias}"
  zle -N self-insert twocp_self_insert
  __twocp_self_insert_saved=1

  __twocp_bind_widget "${TWOCP_KEY_SHOW}" twocp-show-or-refresh
  __twocp_bind_widget "${TWOCP_KEY_ACCEPT}" twocp-accept-suggestion
  __twocp_bind_widget "${TWOCP_KEY_ENTER}" twocp-accept-or-original
  __twocp_bind_widget "${TWOCP_KEY_ENTER_ALT}" twocp-accept-or-original
  __twocp_bind_widget "${TWOCP_KEY_DISMISS}" twocp-dismiss-suggestions
  __twocp_bind_widget "${TWOCP_KEY_NEXT}" twocp-next-suggestion
  __twocp_bind_widget "${TWOCP_KEY_PREVIOUS}" twocp-previous-suggestion
  __twocp_bind_widget "${TWOCP_KEY_DOWN}" twocp-down-or-original
  __twocp_bind_widget "${TWOCP_KEY_UP}" twocp-up-or-original
  __twocp_bind_widget "${TWOCP_KEY_DOWN_ALT}" twocp-down-or-original
  __twocp_bind_widget "${TWOCP_KEY_UP_ALT}" twocp-up-or-original
  if [[ -n "${terminfo[kcud1]-}" ]]; then
    __twocp_bind_widget "${terminfo[kcud1]}" twocp-down-or-original
  fi
  if [[ -n "${terminfo[kcuu1]-}" ]]; then
    __twocp_bind_widget "${terminfo[kcuu1]}" twocp-up-or-original
  fi
  __twocp_bind_widget ' ' twocp-space-maybe-show

  add-zsh-hook precmd __twocp_precmd_clear
  add-zsh-hook preexec __twocp_precmd_clear

  __twocp_enabled=1
}

function twocp_zsh_disable() {
  if (( ! __twocp_enabled )); then
    return 0
  fi

  __twocp_restore_binding "${TWOCP_KEY_SHOW}"
  __twocp_restore_binding "${TWOCP_KEY_ACCEPT}"
  __twocp_restore_binding "${TWOCP_KEY_ENTER}"
  __twocp_restore_binding "${TWOCP_KEY_ENTER_ALT}"
  __twocp_restore_binding "${TWOCP_KEY_DISMISS}"
  __twocp_restore_binding "${TWOCP_KEY_NEXT}"
  __twocp_restore_binding "${TWOCP_KEY_PREVIOUS}"
  __twocp_restore_binding "${TWOCP_KEY_DOWN}"
  __twocp_restore_binding "${TWOCP_KEY_UP}"
  __twocp_restore_binding "${TWOCP_KEY_DOWN_ALT}"
  __twocp_restore_binding "${TWOCP_KEY_UP_ALT}"
  if [[ -n "${terminfo[kcud1]-}" ]]; then
    __twocp_restore_binding "${terminfo[kcud1]}"
  fi
  if [[ -n "${terminfo[kcuu1]-}" ]]; then
    __twocp_restore_binding "${terminfo[kcuu1]}"
  fi
  __twocp_restore_binding ' '

  if (( __twocp_self_insert_saved )); then
    zle -A "${__twocp_self_insert_alias}" self-insert
    __twocp_self_insert_saved=0
  fi

  add-zsh-hook -d precmd __twocp_precmd_clear 2>/dev/null
  add-zsh-hook -d preexec __twocp_precmd_clear 2>/dev/null

  __twocp_invalidate_menu
  __twocp_enabled=0
}

if [[ -o interactive ]]; then
  twocp_zsh_enable
fi

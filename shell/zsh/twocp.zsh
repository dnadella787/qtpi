autoload -Uz add-zsh-hook add-zle-hook-widget
zmodload zsh/terminfo 2>/dev/null || true

if (( ${+functions[twocp_zsh_disable]} )) && (( ${__twocp_enabled:-0} )); then
  twocp_zsh_disable 2>/dev/null || true
fi

typeset -g TWOCP_BIN="${TWOCP_BIN:-twocp}"
typeset -gi TWOCP_MAX_SUGGESTIONS="${TWOCP_MAX_SUGGESTIONS:-50}"
typeset -g TWOCP_KEY_SHOW="${TWOCP_KEY_SHOW:-^X2s}"
typeset -g TWOCP_KEY_ENTER="${TWOCP_KEY_ENTER:-^M}"
typeset -g TWOCP_KEY_ENTER_ALT="${TWOCP_KEY_ENTER_ALT:-^J}"
typeset -g TWOCP_KEY_ESCAPE="${TWOCP_KEY_ESCAPE:-^[}"
typeset -g TWOCP_KEY_BACKSPACE="${TWOCP_KEY_BACKSPACE:-^?}"
typeset -g TWOCP_KEY_BACKSPACE_ALT="${TWOCP_KEY_BACKSPACE_ALT:-^H}"
typeset -g TWOCP_KEY_DOWN="${TWOCP_KEY_DOWN:-^[[B}"
typeset -g TWOCP_KEY_UP="${TWOCP_KEY_UP:-^[[A}"
typeset -g TWOCP_KEY_DOWN_ALT="${TWOCP_KEY_DOWN_ALT:-^[OB}"
typeset -g TWOCP_KEY_UP_ALT="${TWOCP_KEY_UP_ALT:-^[OA}"
typeset -g TWOCP_KEY_INTERRUPT="${TWOCP_KEY_INTERRUPT:-^C}"
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
typeset -gi __twocp_autosuggest_suppressed=0
typeset -gi __twocp_menu_visible=0
typeset -gi __twocp_selection_index=-1
typeset -gi __twocp_selection_touched=0
typeset -gi __twocp_replace_start=0
typeset -gi __twocp_replace_end=0
typeset -gi __twocp_request_cursor=0
typeset -gi __twocp_truncated_count=0
typeset -gi __twocp_lookup_count=0
typeset -gi __twocp_lookup_time_ms=0
typeset -gi __twocp_after_widget_fd=-1
typeset -g __twocp_after_widget_pid=''
typeset -gi __twocp_overlay_supported=-1
typeset -gi __twocp_overlay_highlight_supported=0
typeset -g __twocp_overlay_highlight_enter=''
typeset -g __twocp_overlay_highlight_exit=''
typeset -gi __twocp_overlay_visible_rows=5
typeset -gi __twocp_overlay_window_start=0
typeset -gi __twocp_overlay_origin_row=0
typeset -gi __twocp_rendered_row_count=0
typeset -gi __twocp_skip_next_pre_redraw_clear=0
typeset -gi __twocp_transient_bindings_active=0
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
  __twocp_selection_index=-1
  __twocp_selection_touched=0
  __twocp_replace_start=0
  __twocp_replace_end=0
  __twocp_request_cursor=0
  __twocp_truncated_count=0
  __twocp_lookup_count=0
  __twocp_lookup_time_ms=0
  __twocp_reset_overlay_state
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
}

function __twocp_set_status_message() {
  :
}

function __twocp_buffer_has_content() {
  [[ -n "${BUFFER//[[:space:]]/}" ]]
}

function __twocp_reset_overlay_state() {
  __twocp_overlay_window_start=0
  __twocp_overlay_origin_row=0
  __twocp_rendered_row_count=0
  __twocp_skip_next_pre_redraw_clear=0
  __twocp_overlay_highlight_supported=0
  __twocp_overlay_highlight_enter=''
  __twocp_overlay_highlight_exit=''
}

function __twocp_suggestion_count() {
  REPLY="${#__twocp_insert_texts[@]}"
}

function __twocp_overlay_max_window_start() {
  local total="$1"
  local max_window_start=$(( total - __twocp_overlay_visible_rows ))

  if (( max_window_start < 0 )); then
    max_window_start=0
  fi

  REPLY="${max_window_start}"
}

function __twocp_overlay_window_end_index() {
  local total="$1"
  local window_end=$(( __twocp_overlay_window_start + __twocp_overlay_visible_rows ))

  if (( window_end > total )); then
    window_end=${total}
  fi

  REPLY="${window_end}"
}

function __twocp_begin_menu_refresh() {
  local total=${#__twocp_insert_texts[@]}

  if (( total == 0 )); then
    __twocp_menu_visible=0
    __twocp_selection_index=-1
    __twocp_overlay_window_start=0
    return 1
  fi

  __twocp_menu_visible=1
  __twocp_selection_index=0
  __twocp_selection_touched=0
  __twocp_overlay_window_start=0
  return 0
}

function __twocp_move_selection_down() {
  local total=${#__twocp_insert_texts[@]}

  if (( ! __twocp_menu_visible || total == 0 )); then
    return 1
  fi

  if (( __twocp_selection_index < total - 1 )); then
    __twocp_selection_index=$(( __twocp_selection_index + 1 ))
    __twocp_selection_touched=1
    if (( __twocp_selection_index >= __twocp_overlay_window_start + __twocp_overlay_visible_rows )); then
      __twocp_overlay_window_start=$(( __twocp_selection_index - __twocp_overlay_visible_rows + 1 ))
    fi
  fi

  return 0
}

function __twocp_move_selection_up() {
  local total=${#__twocp_insert_texts[@]}

  if (( ! __twocp_menu_visible || total == 0 )); then
    return 1
  fi

  if (( __twocp_selection_index > 0 )); then
    __twocp_selection_index=$(( __twocp_selection_index - 1 ))
    __twocp_selection_touched=1
    if (( __twocp_selection_index < __twocp_overlay_window_start )); then
      __twocp_overlay_window_start=${__twocp_selection_index}
    fi
  fi

  return 0
}

function __twocp_menu_state_valid() {
  local total=${#__twocp_insert_texts[@]}

  if (( ! __twocp_menu_visible || total == 0 )); then
    return 1
  fi

  if (( __twocp_selection_index < 0 || __twocp_selection_index >= total )); then
    return 1
  fi

  __twocp_overlay_max_window_start "${total}"
  local max_window_start="${REPLY}"
  if (( __twocp_overlay_window_start < 0 || __twocp_overlay_window_start > max_window_start )); then
    return 1
  fi

  __twocp_overlay_window_end_index "${total}"
  local window_end="${REPLY}"
  if (( __twocp_selection_index < __twocp_overlay_window_start || __twocp_selection_index >= window_end )); then
    return 1
  fi

  return 0
}

function __twocp_clear_prompt_state() {
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

function __twocp_restore_menu_paint() {
  __twocp_clear_overlay
  __twocp_deactivate_transient_bindings
  __twocp_clear_prompt_state
  __twocp_set_status_message ''
  __twocp_enable_autosuggestions
}

function __twocp_invalidate_menu() {
  __twocp_cancel_after_widget_refresh
  __twocp_restore_menu_paint
  __twocp_reset_state
}

function __twocp_query_suggestions() {
  if ! __twocp_buffer_has_content; then
    return 1
  fi

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
      return 1
    }

  eval "${response}"
  __twocp_debug "refresh:done status=${__twocp_status} count=${#__twocp_insert_texts[@]} request=${(qqq)__twocp_request_buffer} cursor=${__twocp_request_cursor}"

  return 0
}

function __twocp_init_overlay_highlight_mode() {
  __twocp_overlay_highlight_supported=0
  __twocp_overlay_highlight_enter=''
  __twocp_overlay_highlight_exit=''

  if [[ -n "${terminfo[smso]-}" && -n "${terminfo[rmso]-}" ]]; then
    __twocp_overlay_highlight_supported=1
    __twocp_overlay_highlight_enter="${terminfo[smso]}"
    __twocp_overlay_highlight_exit="${terminfo[rmso]}"
    __twocp_debug "overlay:highlight-mode standout"
  elif [[ -n "${terminfo[rev]-}" && -n "${terminfo[sgr0]-}" ]]; then
    __twocp_overlay_highlight_supported=1
    __twocp_overlay_highlight_enter="${terminfo[rev]}"
    __twocp_overlay_highlight_exit="${terminfo[sgr0]}"
    __twocp_debug "overlay:highlight-mode reverse"
  else
    __twocp_debug "overlay:highlight-mode marker"
  fi
}

function __twocp_overlay_capable() {
  if (( __twocp_overlay_supported < 0 )); then
    __twocp_overlay_supported=1

    if [[ "${TERM:-}" == 'dumb' ]]; then
      __twocp_debug "overlay:term-dumb"
      __twocp_overlay_supported=0
      return 1
    fi

    local capability
    for capability in sc rc cup el; do
      if [[ -z "${terminfo[$capability]-}" ]]; then
        __twocp_debug "overlay:missing-capability ${capability}"
        __twocp_overlay_supported=0
        return 1
      fi
    done
  fi

  if (( __twocp_overlay_supported == 0 )); then
    return 1
  fi

  if (( ! __twocp_overlay_highlight_supported )) \
    && [[ -z "${__twocp_overlay_highlight_enter}" ]] \
    && [[ -z "${__twocp_overlay_highlight_exit}" ]]; then
    __twocp_init_overlay_highlight_mode
  fi

  return 0
}

function __twocp_disable_overlay_session() {
  __twocp_debug "overlay:disable-session"
  __twocp_overlay_supported=0
  __twocp_invalidate_menu
}

function __twocp_query_terminal_cursor() {
  if ! __twocp_overlay_capable; then
    return 1
  fi

  local tty_fd reply='' char='' attempts=0

  exec {tty_fd}<> /dev/tty 2>/dev/null || {
    __twocp_debug "overlay:tty-open-failed"
    return 1
  }

  print -rn -- $'\e[6n' >&${tty_fd}
  while (( attempts < 32 )); do
    if ! read -r -u ${tty_fd} -k 1 -t 0.05 char 2>/dev/null; then
      break
    fi
    reply+="${char}"
    if [[ "${char}" == 'R' ]]; then
      break
    fi
    attempts=$(( attempts + 1 ))
  done

  exec {tty_fd}<&- 2>/dev/null || true
  exec {tty_fd}>&- 2>/dev/null || true

  if [[ ! "${reply}" =~ $'^\x1b\\[([0-9]+);([0-9]+)R$' ]]; then
    __twocp_debug "overlay:cursor-query-failed reply=${(qqq)reply}"
    return 1
  fi

  REPLY="${match[1]} ${match[2]}"
  return 0
}

function __twocp_overlay_format_rows() {
  local total=${#__twocp_insert_texts[@]}
  local available_columns=${COLUMNS:-80}
  local window_start=${__twocp_overlay_window_start}
  local window_end=0
  local display_width=0
  local display_len
  local annotation_budget=0
  local prefix_width=2
  local content_budget=0
  local row_budget=0
  local index display annotation display_row prefix selected

  REPLY=''
  if (( total == 0 || available_columns <= 0 )); then
    return 1
  fi

  __twocp_overlay_window_end_index "${total}"
  window_end="${REPLY}"

  content_budget=$(( available_columns - prefix_width ))
  if (( content_budget < 16 )); then
    content_budget=16
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

  local -a rows=()
  for (( index = window_start + 1; index <= window_end; index += 1 )); do
    display="${__twocp_displays[index]}"
    annotation="${__twocp_annotations[index]}"
    selected=0

    if (( index - 1 == __twocp_selection_index )); then
      selected=1
    fi

    if (( ${#display} > display_width )) && (( display_width > 3 )); then
      display="${display[1,$(( display_width - 3 ))]}..."
    fi
    display="${(r:display_width:: :)display}"

    if (( selected )) && (( ! __twocp_overlay_highlight_supported )); then
      prefix='> '
    else
      prefix='  '
    fi

    if [[ -n "${annotation}" ]] && (( annotation_budget > 0 )); then
      if (( ${#annotation} > annotation_budget )); then
        if (( annotation_budget > 3 )); then
          annotation="${annotation[1,$(( annotation_budget - 3 ))]}..."
        else
          annotation=''
        fi
      fi
    else
      annotation=''
    fi

    if [[ -n "${annotation}" ]]; then
      display_row="${prefix}${display} | ${annotation}"
    else
      display_row="${prefix}${display}"
    fi

    row_budget=${available_columns}
    if (( ${#display_row} > row_budget )) && (( row_budget > 3 )); then
      display_row="${display_row[1,$(( row_budget - 3 ))]}..."
    fi
    rows+=("${display_row}")
  done

  REPLY="${(pj:\n:)rows}"
  return 0
}

function __twocp_clear_overlay() {
  if (( __twocp_rendered_row_count == 0 || __twocp_overlay_origin_row <= 0 )); then
    return 0
  fi

  if ! __twocp_overlay_capable; then
    __twocp_overlay_origin_row=0
    __twocp_rendered_row_count=0
    __twocp_skip_next_pre_redraw_clear=0
    return 0
  fi

  local tty_fd row=0
  exec {tty_fd}> /dev/tty 2>/dev/null || {
    __twocp_overlay_origin_row=0
    __twocp_rendered_row_count=0
    __twocp_skip_next_pre_redraw_clear=0
    return 0
  }

  print -rn -- "${terminfo[sc]}" >&${tty_fd}
  for (( row = 0; row < __twocp_rendered_row_count; row += 1 )); do
    echoti cup $(( __twocp_overlay_origin_row - 1 + row )) 0 >&${tty_fd}
    print -rn -- "${terminfo[el]}" >&${tty_fd}
  done
  print -rn -- "${terminfo[rc]}" >&${tty_fd}

  exec {tty_fd}>&- 2>/dev/null || true
  __twocp_overlay_origin_row=0
  __twocp_rendered_row_count=0
  __twocp_skip_next_pre_redraw_clear=0
}

function __twocp_render_overlay() {
  if ! __twocp_overlay_capable; then
    return 1
  fi

  if ! __twocp_menu_state_valid; then
    return 1
  fi

  if ! __twocp_overlay_format_rows; then
    return 1
  fi

  local rows="${REPLY}"
  local -a formatted_rows=("${(@f)rows}")
  if (( ${#formatted_rows[@]} == 0 )); then
    return 1
  fi

  local window_start=${__twocp_overlay_window_start}

  if ! __twocp_query_terminal_cursor; then
    return 1
  fi

  local cursor_row cursor_col
  cursor_row=${${=REPLY}[1]}
  cursor_col=${${=REPLY}[2]}

  local columns=${COLUMNS:-80}
  if (( columns <= 0 )); then
    return 1
  fi

  local remainder_chars=$(( ${#BUFFER} - CURSOR ))
  if (( remainder_chars < 0 )); then
    remainder_chars=0
  fi

  local rows_to_buffer_end=$(( (cursor_col - 1 + remainder_chars) / columns ))
  local overlay_row=$(( cursor_row + rows_to_buffer_end + 1 ))
  local tty_fd row_index=1 row_text=''

  __twocp_clear_overlay

  exec {tty_fd}> /dev/tty 2>/dev/null || return 1
  print -rn -- "${terminfo[sc]}" >&${tty_fd}
  for (( row_index = 1; row_index <= ${#formatted_rows[@]}; row_index += 1 )); do
    row_text="${formatted_rows[row_index]}"
    echoti cup $(( overlay_row - 1 + row_index - 1 )) 0 >&${tty_fd}
    print -rn -- "${terminfo[el]}" >&${tty_fd}
    if (( __twocp_overlay_highlight_supported )) && (( window_start + row_index - 1 == __twocp_selection_index )); then
      print -rn -- "${__twocp_overlay_highlight_enter}${row_text}${__twocp_overlay_highlight_exit}" >&${tty_fd}
    else
      print -rn -- "${row_text}" >&${tty_fd}
    fi
  done
  print -rn -- "${terminfo[rc]}" >&${tty_fd}
  exec {tty_fd}>&- 2>/dev/null || true

  __twocp_overlay_origin_row=${overlay_row}
  __twocp_rendered_row_count=${#formatted_rows[@]}
  __twocp_skip_next_pre_redraw_clear=1
  __twocp_debug "overlay:render rows=${__twocp_rendered_row_count} origin=${__twocp_overlay_origin_row} selection=${__twocp_selection_index} highlight=${__twocp_overlay_highlight_supported}"
  return 0
}

function __twocp_refresh_now() {
  __twocp_cancel_after_widget_refresh

  if ! __twocp_query_suggestions; then
    __twocp_invalidate_menu
    return
  fi

  if ! __twocp_begin_menu_refresh; then
    __twocp_invalidate_menu
    return
  fi

  if ! __twocp_overlay_capable; then
    __twocp_disable_overlay_session
    return
  fi

  __twocp_activate_transient_bindings
  __twocp_disable_autosuggestions
  if ! __twocp_render_overlay; then
    __twocp_disable_overlay_session
    return
  fi
}

function __twocp_after_widget_ready() {
  local fd="$1"
  local discard=''

  __twocp_debug "after-widget:ready buffer=${(qqq)BUFFER} cursor=${CURSOR}"
  zle -F "${fd}" 2>/dev/null || true
  read -r -u "${fd}" -k 1 discard 2>/dev/null || true
  exec {fd}<&- 2>/dev/null || true
  __twocp_after_widget_fd=-1
  __twocp_after_widget_pid=''
  __twocp_refresh_now
}

function __twocp_schedule_after_widget_refresh() {
  __twocp_debug "after-widget:schedule-refresh buffer=${(qqq)BUFFER} cursor=${CURSOR}"
  __twocp_cancel_after_widget_refresh
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

  if (( __twocp_selection_index < 0 || __twocp_selection_index >= ${#__twocp_insert_texts[@]} )); then
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
  return 0
}

function __twocp_enter_should_accept_selection() {
  if ! __twocp_menu_current; then
    return 1
  fi

  if (( __twocp_rendered_row_count == 0 )); then
    return 1
  fi

  if (( __twocp_selection_index < 0 || __twocp_selection_index >= ${#__twocp_insert_texts[@]} )); then
    return 1
  fi

  if (( __twocp_replace_start == __twocp_replace_end )) && (( ! __twocp_selection_touched )); then
    return 1
  fi

  return 0
}

function twocp_show_or_refresh() {
  __twocp_refresh_now
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

function twocp_interrupt_or_original() {
  __twocp_invalidate_menu
  __twocp_call_saved_widget "${KEYS}"
}

function twocp_next_suggestion() {
  if ! __twocp_menu_current; then
    return 0
  fi

  __twocp_cancel_after_widget_refresh
  __twocp_move_selection_down || return 0
  __twocp_render_overlay || __twocp_disable_overlay_session
}

function twocp_previous_suggestion() {
  if ! __twocp_menu_current; then
    return 0
  fi

  __twocp_cancel_after_widget_refresh
  __twocp_move_selection_up || return 0
  __twocp_render_overlay || __twocp_disable_overlay_session
}

function __twocp_call_saved_widget() {
  local key="$1"
  local preserve_menu="${2:-0}"
  local keymap="${KEYMAP:-main}"
  local managed_widget="${WIDGET:-}"
  local binding_id="$(__twocp_binding_id "${keymap}" "${key}")"
  local widget_binding_id="$(__twocp_binding_id "${keymap}" "${managed_widget}")"
  local saved="${__twocp_saved_bindings[$binding_id]-}"
  local saved_alias="${__twocp_saved_widget_aliases[$binding_id]-}"
  local widget_saved="${__twocp_saved_widgets_by_widget[$widget_binding_id]-}"
  local widget_saved_alias="${__twocp_saved_widget_aliases_by_widget[$widget_binding_id]-}"
  local use_alias=1
  local had_menu=${__twocp_menu_visible}

  __twocp_debug "saved-widget:start widget=${managed_widget} keymap=${keymap} key=${(qqq)key} saved=${saved} alias=${saved_alias} widget_saved=${widget_saved} widget_alias=${widget_saved_alias} buffer=${(qqq)BUFFER}"
  __twocp_cancel_after_widget_refresh
  if (( had_menu )) && (( ! preserve_menu )); then
    __twocp_invalidate_menu
  fi
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
    twocp-space-maybe-show)
      print -r -- self-insert
      return 0
      ;;
    twocp-backspace-or-original)
      print -r -- backward-delete-char
      return 0
      ;;
    twocp-interrupt-or-original)
      print -r -- send-break
      return 0
      ;;
  esac

  print -r -- "${saved_widget}"
}

function twocp_down_or_original() {
  if (( __twocp_menu_visible )); then
    twocp_next_suggestion
    return 0
  fi

  __twocp_call_saved_widget "${KEYS}"
}

function twocp_up_or_original() {
  if (( __twocp_menu_visible )); then
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

function __twocp_call_saved_self_insert() {
  if (( __twocp_self_insert_saved )); then
    zle "${__twocp_self_insert_alias}" 2>/dev/null && return 0
  fi

  zle .self-insert
}

function twocp_self_insert() {
  __twocp_cancel_after_widget_refresh
  __twocp_call_saved_self_insert
  if __twocp_should_auto_refresh_after_space; then
    __twocp_schedule_after_widget_refresh
  elif (( __twocp_menu_visible )); then
    __twocp_schedule_after_widget_refresh
  fi
}

function twocp_space_maybe_show() {
  __twocp_call_saved_widget "${KEYS}" 1
  if __twocp_should_auto_refresh_after_space; then
    __twocp_schedule_after_widget_refresh
  elif (( __twocp_menu_visible )); then
    __twocp_schedule_after_widget_refresh
  fi
}

function twocp_backspace_or_original() {
  local was_visible=${__twocp_menu_visible}

  __twocp_call_saved_widget "${KEYS}" 1

  if (( was_visible )); then
    if ! __twocp_buffer_has_content; then
      __twocp_invalidate_menu
    else
      __twocp_schedule_after_widget_refresh
    fi
  fi
}

function __twocp_activate_transient_bindings() {
  if (( __twocp_transient_bindings_active )); then
    return 0
  fi

  zle -A self-insert "${__twocp_self_insert_alias}"
  zle -N self-insert twocp_self_insert
  __twocp_self_insert_saved=1
  __twocp_bind_widget "${TWOCP_KEY_ENTER}" twocp-accept-or-original
  __twocp_bind_widget "${TWOCP_KEY_ENTER_ALT}" twocp-accept-or-original
  __twocp_bind_widget "${TWOCP_KEY_ESCAPE}" twocp-dismiss-suggestions
  __twocp_bind_widget "${TWOCP_KEY_DOWN}" twocp-down-or-original
  __twocp_bind_widget "${TWOCP_KEY_UP}" twocp-up-or-original
  __twocp_bind_widget "${TWOCP_KEY_DOWN_ALT}" twocp-down-or-original
  __twocp_bind_widget "${TWOCP_KEY_UP_ALT}" twocp-up-or-original
  __twocp_bind_widget "${TWOCP_KEY_BACKSPACE}" twocp-backspace-or-original
  __twocp_bind_widget "${TWOCP_KEY_BACKSPACE_ALT}" twocp-backspace-or-original
  __twocp_bind_widget "${TWOCP_KEY_INTERRUPT}" twocp-interrupt-or-original
  if [[ -n "${terminfo[kcud1]-}" ]]; then
    __twocp_bind_widget "${terminfo[kcud1]}" twocp-down-or-original
  fi
  if [[ -n "${terminfo[kcuu1]-}" ]]; then
    __twocp_bind_widget "${terminfo[kcuu1]}" twocp-up-or-original
  fi
  if [[ -n "${terminfo[kbs]-}" ]]; then
    __twocp_bind_widget "${terminfo[kbs]}" twocp-backspace-or-original
  fi
  __twocp_transient_bindings_active=1
}

function __twocp_deactivate_transient_bindings() {
  if (( ! __twocp_transient_bindings_active )); then
    return 0
  fi

  __twocp_restore_binding "${TWOCP_KEY_ENTER}"
  __twocp_restore_binding "${TWOCP_KEY_ENTER_ALT}"
  __twocp_restore_binding "${TWOCP_KEY_ESCAPE}"
  __twocp_restore_binding "${TWOCP_KEY_DOWN}"
  __twocp_restore_binding "${TWOCP_KEY_UP}"
  __twocp_restore_binding "${TWOCP_KEY_DOWN_ALT}"
  __twocp_restore_binding "${TWOCP_KEY_UP_ALT}"
  __twocp_restore_binding "${TWOCP_KEY_BACKSPACE}"
  __twocp_restore_binding "${TWOCP_KEY_BACKSPACE_ALT}"
  __twocp_restore_binding "${TWOCP_KEY_INTERRUPT}"
  if [[ -n "${terminfo[kcud1]-}" ]]; then
    __twocp_restore_binding "${terminfo[kcud1]}"
  fi
  if [[ -n "${terminfo[kcuu1]-}" ]]; then
    __twocp_restore_binding "${terminfo[kcuu1]}"
  fi
  if [[ -n "${terminfo[kbs]-}" ]]; then
    __twocp_restore_binding "${terminfo[kbs]}"
  fi
  if (( __twocp_self_insert_saved )); then
    zle -A "${__twocp_self_insert_alias}" self-insert
    __twocp_self_insert_saved=0
  fi
  __twocp_transient_bindings_active=0
}

function __twocp_precmd_clear() {
  __twocp_cancel_after_widget_refresh
  __twocp_restore_menu_paint
  __twocp_reset_state
}

function __twocp_line_init_cleanup() {
  __twocp_clear_prompt_state
  __twocp_set_status_message ''
  __twocp_enable_autosuggestions
}

function __twocp_line_pre_redraw_cleanup() {
  if (( __twocp_skip_next_pre_redraw_clear )); then
    __twocp_skip_next_pre_redraw_clear=0
    __twocp_debug "overlay:skip-pre-redraw-clear"
    return 0
  fi

  if (( __twocp_menu_visible )) && (( __twocp_after_widget_fd >= 0 )); then
    __twocp_debug "overlay:skip-pre-redraw-clear-pending-refresh"
    return 0
  fi

  __twocp_debug "overlay:pre-redraw-clear visible=${__twocp_menu_visible} rows=${__twocp_rendered_row_count}"
  __twocp_clear_overlay
}

function __twocp_line_finish_cleanup() {
  __twocp_cancel_after_widget_refresh
  __twocp_restore_menu_paint
  __twocp_reset_state
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
    twocp-interrupt-or-original)
      print -r -- send-break
      ;;
    twocp-down-or-original)
      print -r -- down-line-or-history
      ;;
    twocp-up-or-original)
      print -r -- up-line-or-history
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
  zle -N twocp-interrupt-or-original twocp_interrupt_or_original
  zle -N twocp-next-suggestion twocp_next_suggestion
  zle -N twocp-previous-suggestion twocp_previous_suggestion
  zle -N twocp-down-or-original twocp_down_or_original
  zle -N twocp-up-or-original twocp_up_or_original
  zle -N twocp-space-maybe-show twocp_space_maybe_show
  zle -N twocp-backspace-or-original twocp_backspace_or_original
  zle -N twocp-self-insert twocp_self_insert
  zle -N __twocp-after-widget-ready __twocp_after_widget_ready
  zle -N __twocp-line-pre-redraw-cleanup __twocp_line_pre_redraw_cleanup
  zle -N __twocp-line-finish-cleanup __twocp_line_finish_cleanup

  __twocp_bind_widget "${TWOCP_KEY_SHOW}" twocp-show-or-refresh
  __twocp_bind_widget ' ' twocp-space-maybe-show

  add-zsh-hook precmd __twocp_precmd_clear
  add-zsh-hook preexec __twocp_precmd_clear
  add-zle-hook-widget line-pre-redraw __twocp-line-pre-redraw-cleanup
  add-zle-hook-widget line-finish __twocp-line-finish-cleanup

  __twocp_enabled=1
}

function twocp_zsh_disable() {
  if (( ! __twocp_enabled )); then
    return 0
  fi

  __twocp_restore_binding "${TWOCP_KEY_SHOW}"
  __twocp_restore_binding ' '

  add-zsh-hook -d precmd __twocp_precmd_clear 2>/dev/null
  add-zsh-hook -d preexec __twocp_precmd_clear 2>/dev/null
  add-zle-hook-widget -d line-pre-redraw __twocp-line-pre-redraw-cleanup 2>/dev/null
  add-zle-hook-widget -d line-finish __twocp-line-finish-cleanup 2>/dev/null

  __twocp_deactivate_transient_bindings
  __twocp_invalidate_menu
  __twocp_enabled=0
}

if [[ -o interactive ]]; then
  twocp_zsh_enable
fi

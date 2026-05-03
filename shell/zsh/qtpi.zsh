autoload -Uz add-zsh-hook add-zle-hook-widget
zmodload zsh/terminfo 2>/dev/null || true

if (( ${+functions[qtpi_zsh_disable]} )) && (( ${__qtpi_enabled:-0} )); then
  qtpi_zsh_disable 2>/dev/null || true
fi

typeset -g QTPI_BIN="${QTPI_BIN:-qtpi}"
typeset -gi QTPI_MAX_SUGGESTIONS="${QTPI_MAX_SUGGESTIONS:-50}"
typeset -g QTPI_KEY_SHOW="${QTPI_KEY_SHOW:-^X2s}"
typeset -g QTPI_KEY_ENTER="${QTPI_KEY_ENTER:-^M}"
typeset -g QTPI_KEY_ENTER_ALT="${QTPI_KEY_ENTER_ALT:-^J}"
typeset -g QTPI_KEY_ESCAPE="${QTPI_KEY_ESCAPE:-^[}"
typeset -g QTPI_KEY_BACKSPACE="${QTPI_KEY_BACKSPACE:-^?}"
typeset -g QTPI_KEY_BACKSPACE_ALT="${QTPI_KEY_BACKSPACE_ALT:-^H}"
typeset -g QTPI_KEY_DOWN="${QTPI_KEY_DOWN:-^[[B}"
typeset -g QTPI_KEY_UP="${QTPI_KEY_UP:-^[[A}"
typeset -g QTPI_KEY_DOWN_ALT="${QTPI_KEY_DOWN_ALT:-^[OB}"
typeset -g QTPI_KEY_UP_ALT="${QTPI_KEY_UP_ALT:-^[OA}"
typeset -g QTPI_KEY_INTERRUPT="${QTPI_KEY_INTERRUPT:-^C}"
typeset -g QTPI_AUTO_ROOTS="${QTPI_AUTO_ROOTS:-git kubectl k}"
typeset -g QTPI_DEBUG_LOG="${QTPI_DEBUG_LOG:-}"

typeset -g __qtpi_enabled=0
typeset -g __qtpi_status='no_match'
typeset -g __qtpi_provider_id=''
typeset -g __qtpi_parser_status=''
typeset -g __qtpi_request_buffer=''
typeset -g __qtpi_dynamic_slot_id=''
typeset -g __qtpi_lookup_status='not_checked'
typeset -g __qtpi_cache_status='not_checked'
typeset -gi __qtpi_autosuggest_suppressed=0
typeset -gi __qtpi_menu_visible=0
typeset -gi __qtpi_selection_index=-1
typeset -gi __qtpi_selection_touched=0
typeset -gi __qtpi_replace_start=0
typeset -gi __qtpi_replace_end=0
typeset -gi __qtpi_request_cursor=0
typeset -gi __qtpi_truncated_count=0
typeset -gi __qtpi_lookup_count=0
typeset -gi __qtpi_lookup_time_ms=0
typeset -gi __qtpi_after_widget_fd=-1
typeset -g __qtpi_after_widget_pid=''
typeset -gi __qtpi_overlay_supported=-1
typeset -gi __qtpi_overlay_highlight_supported=0
typeset -g __qtpi_overlay_highlight_enter=''
typeset -g __qtpi_overlay_highlight_exit=''
typeset -gi __qtpi_overlay_visible_rows=5
typeset -gi __qtpi_overlay_window_start=0
typeset -gi __qtpi_overlay_origin_row=0
typeset -gi __qtpi_rendered_row_count=0
typeset -gi __qtpi_skip_next_pre_redraw_clear=0
typeset -gi __qtpi_transient_bindings_active=0
typeset -ga __qtpi_insert_texts=()
typeset -ga __qtpi_displays=()
typeset -ga __qtpi_annotations=()
typeset -ga __qtpi_kinds=()
typeset -gA __qtpi_saved_bindings=()
typeset -gA __qtpi_managed_bindings=()
typeset -gA __qtpi_saved_widget_aliases=()
typeset -gA __qtpi_saved_widgets_by_widget=()
typeset -gA __qtpi_saved_widget_aliases_by_widget=()
typeset -g __qtpi_self_insert_saved=0
typeset -g __qtpi_self_insert_alias='__qtpi-orig-self-insert'

function __qtpi_reset_state() {
  __qtpi_status='no_match'
  __qtpi_provider_id=''
  __qtpi_parser_status=''
  __qtpi_request_buffer=''
  __qtpi_dynamic_slot_id=''
  __qtpi_lookup_status='not_checked'
  __qtpi_cache_status='not_checked'
  __qtpi_menu_visible=0
  __qtpi_selection_index=-1
  __qtpi_selection_touched=0
  __qtpi_replace_start=0
  __qtpi_replace_end=0
  __qtpi_request_cursor=0
  __qtpi_truncated_count=0
  __qtpi_lookup_count=0
  __qtpi_lookup_time_ms=0
  __qtpi_reset_overlay_state
  __qtpi_insert_texts=()
  __qtpi_displays=()
  __qtpi_annotations=()
  __qtpi_kinds=()
}

function __qtpi_debug() {
  if [[ -n "${QTPI_DEBUG_LOG}" ]]; then
    print -r -- "$*" >>| "${QTPI_DEBUG_LOG}" 2>/dev/null || true
  fi
}

function __qtpi_runtime_available() {
  if [[ "${QTPI_BIN}" == */* ]]; then
    if [[ ! -x "${QTPI_BIN}" ]]; then
      __qtpi_debug "runtime:missing-explicit-bin bin=${QTPI_BIN}"
      return 1
    fi
  else
    if ! whence -p "${QTPI_BIN}" >/dev/null 2>&1; then
      __qtpi_debug "runtime:missing-bin bin=${QTPI_BIN}"
      return 1
    fi
  fi

  return 0
}

function __qtpi_cancel_after_widget_refresh() {
  if (( __qtpi_after_widget_fd >= 0 )); then
    zle -F "${__qtpi_after_widget_fd}" 2>/dev/null || true
    exec {__qtpi_after_widget_fd}<&- 2>/dev/null || true
    __qtpi_after_widget_fd=-1
  fi

  if [[ -n "${__qtpi_after_widget_pid}" ]]; then
    kill "${__qtpi_after_widget_pid}" 2>/dev/null || true
    wait "${__qtpi_after_widget_pid}" 2>/dev/null || true
    __qtpi_after_widget_pid=''
  fi
}

function __qtpi_set_status_message() {
  :
}

function __qtpi_buffer_has_content() {
  [[ -n "${BUFFER//[[:space:]]/}" ]]
}

function __qtpi_reset_overlay_state() {
  __qtpi_overlay_window_start=0
  __qtpi_overlay_origin_row=0
  __qtpi_rendered_row_count=0
  __qtpi_skip_next_pre_redraw_clear=0
  __qtpi_overlay_highlight_supported=0
  __qtpi_overlay_highlight_enter=''
  __qtpi_overlay_highlight_exit=''
}

function __qtpi_suggestion_count() {
  REPLY="${#__qtpi_insert_texts[@]}"
}

function __qtpi_overlay_max_window_start() {
  local total="$1"
  local max_window_start=$(( total - __qtpi_overlay_visible_rows ))

  if (( max_window_start < 0 )); then
    max_window_start=0
  fi

  REPLY="${max_window_start}"
}

function __qtpi_overlay_window_end_index() {
  local total="$1"
  local window_end=$(( __qtpi_overlay_window_start + __qtpi_overlay_visible_rows ))

  if (( window_end > total )); then
    window_end=${total}
  fi

  REPLY="${window_end}"
}

function __qtpi_begin_menu_refresh() {
  local total=${#__qtpi_insert_texts[@]}

  if (( total == 0 )); then
    __qtpi_menu_visible=0
    __qtpi_selection_index=-1
    __qtpi_overlay_window_start=0
    return 1
  fi

  __qtpi_menu_visible=1
  __qtpi_selection_index=0
  __qtpi_selection_touched=0
  __qtpi_overlay_window_start=0
  return 0
}

function __qtpi_move_selection_down() {
  local total=${#__qtpi_insert_texts[@]}

  if (( ! __qtpi_menu_visible || total == 0 )); then
    return 1
  fi

  if (( __qtpi_selection_index < total - 1 )); then
    __qtpi_selection_index=$(( __qtpi_selection_index + 1 ))
    __qtpi_selection_touched=1
    if (( __qtpi_selection_index >= __qtpi_overlay_window_start + __qtpi_overlay_visible_rows )); then
      __qtpi_overlay_window_start=$(( __qtpi_selection_index - __qtpi_overlay_visible_rows + 1 ))
    fi
  fi

  return 0
}

function __qtpi_move_selection_up() {
  local total=${#__qtpi_insert_texts[@]}

  if (( ! __qtpi_menu_visible || total == 0 )); then
    return 1
  fi

  if (( __qtpi_selection_index > 0 )); then
    __qtpi_selection_index=$(( __qtpi_selection_index - 1 ))
    __qtpi_selection_touched=1
    if (( __qtpi_selection_index < __qtpi_overlay_window_start )); then
      __qtpi_overlay_window_start=${__qtpi_selection_index}
    fi
  fi

  return 0
}

function __qtpi_menu_state_valid() {
  local total=${#__qtpi_insert_texts[@]}

  if (( ! __qtpi_menu_visible || total == 0 )); then
    return 1
  fi

  if (( __qtpi_selection_index < 0 || __qtpi_selection_index >= total )); then
    return 1
  fi

  __qtpi_overlay_max_window_start "${total}"
  local max_window_start="${REPLY}"
  if (( __qtpi_overlay_window_start < 0 || __qtpi_overlay_window_start > max_window_start )); then
    return 1
  fi

  __qtpi_overlay_window_end_index "${total}"
  local window_end="${REPLY}"
  if (( __qtpi_selection_index < __qtpi_overlay_window_start || __qtpi_selection_index >= window_end )); then
    return 1
  fi

  return 0
}

function __qtpi_clear_prompt_state() {
  :
}

function __qtpi_disable_autosuggestions() {
  if (( __qtpi_autosuggest_suppressed )); then
    return 0
  fi

  if (( ${+functions[_zsh_autosuggest_disable]} )); then
    _zsh_autosuggest_disable
    __qtpi_autosuggest_suppressed=1
  elif (( ${+functions[_zsh_autosuggest_clear]} )); then
    _zsh_autosuggest_clear
  fi
}

function __qtpi_enable_autosuggestions() {
  if (( ! __qtpi_autosuggest_suppressed )); then
    return 0
  fi

  if (( ${+functions[_zsh_autosuggest_enable]} )); then
    _zsh_autosuggest_enable
  fi
  __qtpi_autosuggest_suppressed=0
}

function __qtpi_restore_menu_paint() {
  __qtpi_clear_overlay
  __qtpi_deactivate_transient_bindings
  __qtpi_clear_prompt_state
  __qtpi_set_status_message ''
  __qtpi_enable_autosuggestions
}

function __qtpi_invalidate_menu() {
  __qtpi_cancel_after_widget_refresh
  __qtpi_restore_menu_paint
  __qtpi_reset_state
}

function __qtpi_query_suggestions() {
  if ! __qtpi_buffer_has_content; then
    return 1
  fi

  if ! __qtpi_runtime_available; then
    __qtpi_reset_state
    return 1
  fi

  local response
  __qtpi_debug "refresh:start buffer=${(qqq)BUFFER} cursor=${CURSOR} bin=${QTPI_BIN}"
  response="$("${QTPI_BIN}" suggest \
    --shell zsh \
    --buffer "${BUFFER}" \
    --cursor "${CURSOR}" \
    --cursor-units chars \
    --cwd "${PWD}" \
    --columns "${COLUMNS:-0}" \
    --rows "${LINES:-0}" \
    --max-suggestions "${QTPI_MAX_SUGGESTIONS}" \
    --format zsh 2>/dev/null)" || {
      __qtpi_debug "refresh:error buffer=${(qqq)BUFFER} cursor=${CURSOR} bin=${QTPI_BIN}"
      __qtpi_reset_state
      return 1
    }

  if [[ -z "${response}" ]]; then
    __qtpi_debug "refresh:empty-response"
    __qtpi_reset_state
    return 1
  fi

  if ! eval "${response}"; then
    __qtpi_debug "refresh:eval-error"
    __qtpi_reset_state
    return 1
  fi

  if (( ${#__qtpi_insert_texts[@]} != ${#__qtpi_displays[@]} )) \
    || (( ${#__qtpi_insert_texts[@]} != ${#__qtpi_annotations[@]} )) \
    || (( ${#__qtpi_insert_texts[@]} != ${#__qtpi_kinds[@]} )); then
    __qtpi_debug "refresh:malformed-response count=${#__qtpi_insert_texts[@]} displays=${#__qtpi_displays[@]} annotations=${#__qtpi_annotations[@]} kinds=${#__qtpi_kinds[@]}"
    __qtpi_reset_state
    return 1
  fi

  __qtpi_debug "refresh:done status=${__qtpi_status} count=${#__qtpi_insert_texts[@]} request=${(qqq)__qtpi_request_buffer} cursor=${__qtpi_request_cursor}"

  return 0
}

function __qtpi_init_overlay_highlight_mode() {
  __qtpi_overlay_highlight_supported=0
  __qtpi_overlay_highlight_enter=''
  __qtpi_overlay_highlight_exit=''

  if [[ -n "${terminfo[smso]-}" && -n "${terminfo[rmso]-}" ]]; then
    __qtpi_overlay_highlight_supported=1
    __qtpi_overlay_highlight_enter="${terminfo[smso]}"
    __qtpi_overlay_highlight_exit="${terminfo[rmso]}"
    __qtpi_debug "overlay:highlight-mode standout"
  elif [[ -n "${terminfo[rev]-}" && -n "${terminfo[sgr0]-}" ]]; then
    __qtpi_overlay_highlight_supported=1
    __qtpi_overlay_highlight_enter="${terminfo[rev]}"
    __qtpi_overlay_highlight_exit="${terminfo[sgr0]}"
    __qtpi_debug "overlay:highlight-mode reverse"
  else
    __qtpi_debug "overlay:highlight-mode marker"
  fi
}

function __qtpi_overlay_capable() {
  if (( __qtpi_overlay_supported < 0 )); then
    __qtpi_overlay_supported=1

    if [[ "${TERM:-}" == 'dumb' ]]; then
      __qtpi_debug "overlay:term-dumb"
      __qtpi_overlay_supported=0
      return 1
    fi

    local capability
    for capability in sc rc cup el; do
      if [[ -z "${terminfo[$capability]-}" ]]; then
        __qtpi_debug "overlay:missing-capability ${capability}"
        __qtpi_overlay_supported=0
        return 1
      fi
    done
  fi

  if (( __qtpi_overlay_supported == 0 )); then
    return 1
  fi

  if (( ! __qtpi_overlay_highlight_supported )) \
    && [[ -z "${__qtpi_overlay_highlight_enter}" ]] \
    && [[ -z "${__qtpi_overlay_highlight_exit}" ]]; then
    __qtpi_init_overlay_highlight_mode
  fi

  return 0
}

function __qtpi_disable_overlay_session() {
  __qtpi_debug "overlay:disable-session"
  __qtpi_overlay_supported=0
  __qtpi_invalidate_menu
}

function __qtpi_query_terminal_cursor() {
  if ! __qtpi_overlay_capable; then
    return 1
  fi

  local tty_fd reply='' char='' attempts=0

  exec {tty_fd}<> /dev/tty 2>/dev/null || {
    __qtpi_debug "overlay:tty-open-failed"
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
    __qtpi_debug "overlay:cursor-query-failed reply=${(qqq)reply}"
    return 1
  fi

  REPLY="${match[1]} ${match[2]}"
  return 0
}

function __qtpi_overlay_format_rows() {
  local total=${#__qtpi_insert_texts[@]}
  local available_columns=${COLUMNS:-80}
  local window_start=${__qtpi_overlay_window_start}
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

  __qtpi_overlay_window_end_index "${total}"
  window_end="${REPLY}"

  content_budget=$(( available_columns - prefix_width ))
  if (( content_budget < 16 )); then
    content_budget=16
  fi

  for (( index = window_start + 1; index <= window_end; index += 1 )); do
    display_len=${#__qtpi_displays[index]}
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
    display="${__qtpi_displays[index]}"
    annotation="${__qtpi_annotations[index]}"
    selected=0

    if (( index - 1 == __qtpi_selection_index )); then
      selected=1
    fi

    if (( ${#display} > display_width )) && (( display_width > 3 )); then
      display="${display[1,$(( display_width - 3 ))]}..."
    fi
    display="${(r:display_width:: :)display}"

    if (( selected )) && (( ! __qtpi_overlay_highlight_supported )); then
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

function __qtpi_clear_overlay() {
  if (( __qtpi_rendered_row_count == 0 || __qtpi_overlay_origin_row <= 0 )); then
    return 0
  fi

  if ! __qtpi_overlay_capable; then
    __qtpi_overlay_origin_row=0
    __qtpi_rendered_row_count=0
    __qtpi_skip_next_pre_redraw_clear=0
    return 0
  fi

  local tty_fd row=0
  exec {tty_fd}> /dev/tty 2>/dev/null || {
    __qtpi_overlay_origin_row=0
    __qtpi_rendered_row_count=0
    __qtpi_skip_next_pre_redraw_clear=0
    return 0
  }

  print -rn -- "${terminfo[sc]}" >&${tty_fd}
  for (( row = 0; row < __qtpi_rendered_row_count; row += 1 )); do
    echoti cup $(( __qtpi_overlay_origin_row - 1 + row )) 0 >&${tty_fd}
    print -rn -- "${terminfo[el]}" >&${tty_fd}
  done
  print -rn -- "${terminfo[rc]}" >&${tty_fd}

  exec {tty_fd}>&- 2>/dev/null || true
  __qtpi_overlay_origin_row=0
  __qtpi_rendered_row_count=0
  __qtpi_skip_next_pre_redraw_clear=0
}

function __qtpi_render_overlay() {
  if ! __qtpi_overlay_capable; then
    return 1
  fi

  if ! __qtpi_menu_state_valid; then
    return 1
  fi

  if ! __qtpi_overlay_format_rows; then
    return 1
  fi

  local rows="${REPLY}"
  local -a formatted_rows=("${(@f)rows}")
  if (( ${#formatted_rows[@]} == 0 )); then
    return 1
  fi

  local window_start=${__qtpi_overlay_window_start}

  if ! __qtpi_query_terminal_cursor; then
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

  __qtpi_clear_overlay

  exec {tty_fd}> /dev/tty 2>/dev/null || return 1
  print -rn -- "${terminfo[sc]}" >&${tty_fd}
  for (( row_index = 1; row_index <= ${#formatted_rows[@]}; row_index += 1 )); do
    row_text="${formatted_rows[row_index]}"
    echoti cup $(( overlay_row - 1 + row_index - 1 )) 0 >&${tty_fd}
    print -rn -- "${terminfo[el]}" >&${tty_fd}
    if (( __qtpi_overlay_highlight_supported )) && (( window_start + row_index - 1 == __qtpi_selection_index )); then
      print -rn -- "${__qtpi_overlay_highlight_enter}${row_text}${__qtpi_overlay_highlight_exit}" >&${tty_fd}
    else
      print -rn -- "${row_text}" >&${tty_fd}
    fi
  done
  print -rn -- "${terminfo[rc]}" >&${tty_fd}
  exec {tty_fd}>&- 2>/dev/null || true

  __qtpi_overlay_origin_row=${overlay_row}
  __qtpi_rendered_row_count=${#formatted_rows[@]}
  __qtpi_skip_next_pre_redraw_clear=1
  __qtpi_debug "overlay:render rows=${__qtpi_rendered_row_count} origin=${__qtpi_overlay_origin_row} selection=${__qtpi_selection_index} highlight=${__qtpi_overlay_highlight_supported}"
  return 0
}

function __qtpi_refresh_now() {
  __qtpi_cancel_after_widget_refresh

  if ! __qtpi_query_suggestions; then
    __qtpi_invalidate_menu
    return
  fi

  if ! __qtpi_begin_menu_refresh; then
    __qtpi_invalidate_menu
    return
  fi

  if ! __qtpi_overlay_capable; then
    __qtpi_disable_overlay_session
    return
  fi

  __qtpi_activate_transient_bindings
  __qtpi_disable_autosuggestions
  if ! __qtpi_render_overlay; then
    __qtpi_disable_overlay_session
    return
  fi
}

function __qtpi_after_widget_ready() {
  local fd="$1"
  local discard=''

  __qtpi_debug "after-widget:ready buffer=${(qqq)BUFFER} cursor=${CURSOR}"
  zle -F "${fd}" 2>/dev/null || true
  read -r -u "${fd}" -k 1 discard 2>/dev/null || true
  exec {fd}<&- 2>/dev/null || true
  __qtpi_after_widget_fd=-1
  __qtpi_after_widget_pid=''
  __qtpi_refresh_now
}

function __qtpi_schedule_after_widget_refresh() {
  __qtpi_debug "after-widget:schedule-refresh buffer=${(qqq)BUFFER} cursor=${CURSOR}"
  __qtpi_cancel_after_widget_refresh
  exec {__qtpi_after_widget_fd}< <(
    sleep 0.01
    print -r -- .
  )
  __qtpi_after_widget_pid=$!
  zle -F -w "${__qtpi_after_widget_fd}" __qtpi-after-widget-ready
}

function __qtpi_menu_current() {
  if (( ! __qtpi_menu_visible )); then
    return 1
  fi

  if [[ "${BUFFER}" != "${__qtpi_request_buffer}" ]] || (( CURSOR != __qtpi_request_cursor )); then
    __qtpi_invalidate_menu
    return 1
  fi

  return 0
}

function __qtpi_apply_selection() {
  if ! __qtpi_menu_current; then
    return 1
  fi

  if (( __qtpi_selection_index < 0 || __qtpi_selection_index >= ${#__qtpi_insert_texts[@]} )); then
    return 1
  fi

  local selection=$(( __qtpi_selection_index + 1 ))
  local insert_text="${__qtpi_insert_texts[selection]}"
  local prefix=''
  local suffix=''

  if (( __qtpi_replace_start > 0 )); then
    prefix="${BUFFER[1,__qtpi_replace_start]}"
  fi
  suffix="${BUFFER[__qtpi_replace_end + 1,-1]}"
  BUFFER="${prefix}${insert_text}${suffix}"
  CURSOR=$(( __qtpi_replace_start + ${#insert_text} ))

  __qtpi_invalidate_menu
  if __qtpi_should_auto_refresh_after_space; then
    __qtpi_schedule_after_widget_refresh
  fi
  return 0
}

function __qtpi_enter_should_accept_selection() {
  if ! __qtpi_menu_current; then
    return 1
  fi

  if (( __qtpi_rendered_row_count == 0 )); then
    return 1
  fi

  if (( __qtpi_selection_index < 0 || __qtpi_selection_index >= ${#__qtpi_insert_texts[@]} )); then
    return 1
  fi

  if (( __qtpi_replace_start == __qtpi_replace_end )) && (( ! __qtpi_selection_touched )); then
    return 1
  fi

  return 0
}

function qtpi_show_or_refresh() {
  __qtpi_refresh_now
}

function qtpi_accept_suggestion() {
  __qtpi_apply_selection || true
}

function qtpi_accept_or_original() {
  __qtpi_debug "enter widget=${WIDGET:-} keys=${(qqq)KEYS} keymap=${KEYMAP:-} visible=${__qtpi_menu_visible} buffer=${(qqq)BUFFER}"
  if __qtpi_enter_should_accept_selection; then
    __qtpi_apply_selection || true
    return 0
  fi

  __qtpi_call_saved_widget "${KEYS}"
}

function qtpi_dismiss_suggestions() {
  __qtpi_invalidate_menu
}

function qtpi_interrupt_or_original() {
  __qtpi_invalidate_menu
  __qtpi_call_saved_widget "${KEYS}"
}

function qtpi_next_suggestion() {
  if ! __qtpi_menu_current; then
    return 0
  fi

  __qtpi_cancel_after_widget_refresh
  __qtpi_move_selection_down || return 0
  __qtpi_render_overlay || __qtpi_disable_overlay_session
}

function qtpi_previous_suggestion() {
  if ! __qtpi_menu_current; then
    return 0
  fi

  __qtpi_cancel_after_widget_refresh
  __qtpi_move_selection_up || return 0
  __qtpi_render_overlay || __qtpi_disable_overlay_session
}

function __qtpi_call_saved_widget() {
  local key="$1"
  local preserve_menu="${2:-0}"
  local keymap="${KEYMAP:-main}"
  local managed_widget="${WIDGET:-}"
  local binding_id="$(__qtpi_binding_id "${keymap}" "${key}")"
  local widget_binding_id="$(__qtpi_binding_id "${keymap}" "${managed_widget}")"
  local saved="${__qtpi_saved_bindings[$binding_id]-}"
  local saved_alias="${__qtpi_saved_widget_aliases[$binding_id]-}"
  local widget_saved="${__qtpi_saved_widgets_by_widget[$widget_binding_id]-}"
  local widget_saved_alias="${__qtpi_saved_widget_aliases_by_widget[$widget_binding_id]-}"
  local use_alias=1
  local had_menu=${__qtpi_menu_visible}

  __qtpi_debug "saved-widget:start widget=${managed_widget} keymap=${keymap} key=${(qqq)key} saved=${saved} alias=${saved_alias} widget_saved=${widget_saved} widget_alias=${widget_saved_alias} buffer=${(qqq)BUFFER}"
  __qtpi_cancel_after_widget_refresh
  if (( had_menu )) && (( ! preserve_menu )); then
    __qtpi_invalidate_menu
  fi
  if [[ "${managed_widget}" == 'qtpi-up-or-original' || "${managed_widget}" == 'qtpi-down-or-original' ]]; then
    use_alias=0
  fi
  if (( use_alias )) && [[ -n "${widget_saved_alias}" ]]; then
    __qtpi_debug "saved-widget:widget-alias widget=${widget_saved_alias}"
    zle "${widget_saved_alias}" 2>/dev/null && return 0
  fi
  if (( use_alias )) && [[ -n "${saved_alias}" ]]; then
    __qtpi_debug "saved-widget:alias widget=${saved_alias}"
    zle "${saved_alias}" 2>/dev/null && return 0
  fi
  saved="${widget_saved:-$saved}"
  saved="$(__qtpi_resolve_original_widget "${keymap}" "${managed_widget}" "${saved}")"
  saved="$(__qtpi_normalize_fallback_widget "${managed_widget}" "${saved}")"
  __qtpi_debug "saved-widget:resolved widget=${saved}"

  if [[ -z "${saved}" || "${saved}" == 'undefined-key' ]]; then
    zle .undefined-key 2>/dev/null || true
    return 0
  fi

  zle "${saved}"
}

function __qtpi_normalize_fallback_widget() {
  local managed_widget="$1"
  local saved_widget="$2"

  case "${managed_widget}" in
    qtpi-up-or-original)
      case "${saved_widget}" in
        up-line-or-beginning-search|history-substring-search-up)
          print -r -- up-line-or-history
          return 0
          ;;
      esac
      ;;
    qtpi-down-or-original)
      case "${saved_widget}" in
        down-line-or-beginning-search|history-substring-search-down)
          print -r -- down-line-or-history
          return 0
          ;;
      esac
      ;;
    qtpi-space-maybe-show)
      print -r -- self-insert
      return 0
      ;;
    qtpi-backspace-or-original)
      print -r -- backward-delete-char
      return 0
      ;;
    qtpi-interrupt-or-original)
      print -r -- send-break
      return 0
      ;;
  esac

  print -r -- "${saved_widget}"
}

function qtpi_down_or_original() {
  if (( __qtpi_menu_visible )); then
    qtpi_next_suggestion
    return 0
  fi

  __qtpi_call_saved_widget "${KEYS}"
}

function qtpi_up_or_original() {
  if (( __qtpi_menu_visible )); then
    qtpi_previous_suggestion
    return 0
  fi

  __qtpi_call_saved_widget "${KEYS}"
}

function __qtpi_supported_auto_root() {
  local command="$1"
  local root
  for root in ${(z)QTPI_AUTO_ROOTS}; do
    if [[ "${command}" == "${root}" ]]; then
      return 0
    fi
  done

  return 1
}

function __qtpi_should_auto_refresh_after_space() {
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

  __qtpi_supported_auto_root "${tokens[1]}"
}

function __qtpi_call_saved_self_insert() {
  if (( __qtpi_self_insert_saved )); then
    zle "${__qtpi_self_insert_alias}" 2>/dev/null && return 0
  fi

  zle .self-insert
}

function qtpi_self_insert() {
  __qtpi_cancel_after_widget_refresh
  __qtpi_call_saved_self_insert
  if __qtpi_should_auto_refresh_after_space; then
    __qtpi_schedule_after_widget_refresh
  elif (( __qtpi_menu_visible )); then
    __qtpi_schedule_after_widget_refresh
  fi
}

function qtpi_space_maybe_show() {
  __qtpi_call_saved_widget "${KEYS}" 1
  if __qtpi_should_auto_refresh_after_space; then
    __qtpi_schedule_after_widget_refresh
  elif (( __qtpi_menu_visible )); then
    __qtpi_schedule_after_widget_refresh
  fi
}

function qtpi_backspace_or_original() {
  local was_visible=${__qtpi_menu_visible}

  __qtpi_call_saved_widget "${KEYS}" 1

  if (( was_visible )); then
    if ! __qtpi_buffer_has_content; then
      __qtpi_invalidate_menu
    else
      __qtpi_schedule_after_widget_refresh
    fi
  fi
}

function __qtpi_activate_transient_bindings() {
  if (( __qtpi_transient_bindings_active )); then
    return 0
  fi

  zle -A self-insert "${__qtpi_self_insert_alias}"
  zle -N self-insert qtpi_self_insert
  __qtpi_self_insert_saved=1
  __qtpi_bind_widget "${QTPI_KEY_ENTER}" qtpi-accept-or-original
  __qtpi_bind_widget "${QTPI_KEY_ENTER_ALT}" qtpi-accept-or-original
  __qtpi_bind_widget "${QTPI_KEY_ESCAPE}" qtpi-dismiss-suggestions
  __qtpi_bind_widget "${QTPI_KEY_DOWN}" qtpi-down-or-original
  __qtpi_bind_widget "${QTPI_KEY_UP}" qtpi-up-or-original
  __qtpi_bind_widget "${QTPI_KEY_DOWN_ALT}" qtpi-down-or-original
  __qtpi_bind_widget "${QTPI_KEY_UP_ALT}" qtpi-up-or-original
  __qtpi_bind_widget "${QTPI_KEY_BACKSPACE}" qtpi-backspace-or-original
  __qtpi_bind_widget "${QTPI_KEY_BACKSPACE_ALT}" qtpi-backspace-or-original
  __qtpi_bind_widget "${QTPI_KEY_INTERRUPT}" qtpi-interrupt-or-original
  if [[ -n "${terminfo[kcud1]-}" ]]; then
    __qtpi_bind_widget "${terminfo[kcud1]}" qtpi-down-or-original
  fi
  if [[ -n "${terminfo[kcuu1]-}" ]]; then
    __qtpi_bind_widget "${terminfo[kcuu1]}" qtpi-up-or-original
  fi
  if [[ -n "${terminfo[kbs]-}" ]]; then
    __qtpi_bind_widget "${terminfo[kbs]}" qtpi-backspace-or-original
  fi
  __qtpi_transient_bindings_active=1
}

function __qtpi_deactivate_transient_bindings() {
  if (( ! __qtpi_transient_bindings_active )); then
    return 0
  fi

  __qtpi_restore_binding "${QTPI_KEY_ENTER}"
  __qtpi_restore_binding "${QTPI_KEY_ENTER_ALT}"
  __qtpi_restore_binding "${QTPI_KEY_ESCAPE}"
  __qtpi_restore_binding "${QTPI_KEY_DOWN}"
  __qtpi_restore_binding "${QTPI_KEY_UP}"
  __qtpi_restore_binding "${QTPI_KEY_DOWN_ALT}"
  __qtpi_restore_binding "${QTPI_KEY_UP_ALT}"
  __qtpi_restore_binding "${QTPI_KEY_BACKSPACE}"
  __qtpi_restore_binding "${QTPI_KEY_BACKSPACE_ALT}"
  __qtpi_restore_binding "${QTPI_KEY_INTERRUPT}"
  if [[ -n "${terminfo[kcud1]-}" ]]; then
    __qtpi_restore_binding "${terminfo[kcud1]}"
  fi
  if [[ -n "${terminfo[kcuu1]-}" ]]; then
    __qtpi_restore_binding "${terminfo[kcuu1]}"
  fi
  if [[ -n "${terminfo[kbs]-}" ]]; then
    __qtpi_restore_binding "${terminfo[kbs]}"
  fi
  if (( __qtpi_self_insert_saved )); then
    zle -A "${__qtpi_self_insert_alias}" self-insert
    __qtpi_self_insert_saved=0
  fi
  __qtpi_transient_bindings_active=0
}

function __qtpi_precmd_clear() {
  __qtpi_cancel_after_widget_refresh
  __qtpi_restore_menu_paint
  __qtpi_reset_state
}

function __qtpi_line_init_cleanup() {
  __qtpi_clear_prompt_state
  __qtpi_set_status_message ''
  __qtpi_enable_autosuggestions
}

function __qtpi_line_pre_redraw_cleanup() {
  if (( __qtpi_skip_next_pre_redraw_clear )); then
    __qtpi_skip_next_pre_redraw_clear=0
    __qtpi_debug "overlay:skip-pre-redraw-clear"
    return 0
  fi

  if (( __qtpi_menu_visible )) && (( __qtpi_after_widget_fd >= 0 )); then
    __qtpi_debug "overlay:skip-pre-redraw-clear-pending-refresh"
    return 0
  fi

  __qtpi_debug "overlay:pre-redraw-clear visible=${__qtpi_menu_visible} rows=${__qtpi_rendered_row_count}"
  __qtpi_clear_overlay
}

function __qtpi_line_finish_cleanup() {
  __qtpi_cancel_after_widget_refresh
  __qtpi_restore_menu_paint
  __qtpi_reset_state
}

function __qtpi_widget_for_key() {
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

function __qtpi_keymap_exists() {
  bindkey -M "$1" >/dev/null 2>&1
}

function __qtpi_binding_id() {
  print -r -- "$1:$2"
}

function __qtpi_is_managed_widget() {
  local widget="$1"
  [[ -n "${widget}" && ( "${widget}" == qtpi-* || "${widget}" == __qtpi-after-widget-ready ) ]]
}

function __qtpi_saved_widget_for_managed_widget() {
  local keymap="$1"
  local widget="$2"
  local binding_id saved

  for binding_id in ${(k)__qtpi_managed_bindings}; do
    if [[ "${binding_id}" != "${keymap}:"* ]]; then
      continue
    fi

    if [[ "${__qtpi_managed_bindings[$binding_id]}" != "${widget}" ]]; then
      continue
    fi

    saved="${__qtpi_saved_bindings[$binding_id]-}"
    if [[ -n "${saved}" && "${saved}" != 'undefined-key' ]] && ! __qtpi_is_managed_widget "${saved}"; then
      print -r -- "${saved}"
      return 0
    fi
  done

  return 1
}

function __qtpi_default_widget_for_managed_widget() {
  case "$1" in
    qtpi-accept-or-original)
      print -r -- accept-line
      ;;
    qtpi-interrupt-or-original)
      print -r -- send-break
      ;;
    qtpi-down-or-original)
      print -r -- down-line-or-history
      ;;
    qtpi-up-or-original)
      print -r -- up-line-or-history
      ;;
    *)
      return 1
      ;;
  esac
}

function __qtpi_resolve_original_widget() {
  local keymap="$1"
  local widget="$2"
  local resolved="$3"

  if __qtpi_is_managed_widget "${resolved}"; then
    resolved="$(__qtpi_saved_widget_for_managed_widget "${keymap}" "${resolved}")"
  fi

  if [[ -z "${resolved}" || "${resolved}" == 'undefined-key' ]] && __qtpi_is_managed_widget "${widget}"; then
    resolved="$(__qtpi_default_widget_for_managed_widget "${widget}")"
  fi

  if [[ -n "${resolved}" ]]; then
    print -r -- "${resolved}"
    return 0
  fi

  return 1
}

function __qtpi_bind_widget_in_keymap() {
  local keymap="$1"
  local key="$2"
  local widget="$3"
  local binding_id="$(__qtpi_binding_id "${keymap}" "${key}")"
  local widget_binding_id="$(__qtpi_binding_id "${keymap}" "${widget}")"
  local saved_widget=''
  local saved_alias=''

  if [[ -z "${__qtpi_saved_bindings[$binding_id]+set}" ]]; then
    saved_widget="$(__qtpi_resolve_original_widget "${keymap}" "${widget}" "$(__qtpi_widget_for_key "${key}" "${keymap}")")"
    __qtpi_saved_bindings[$binding_id]="${saved_widget}"
    if [[ -n "${saved_widget}" && "${saved_widget}" != 'undefined-key' ]]; then
      saved_alias="__qtpi-saved-widget-$(( ${#__qtpi_saved_widget_aliases[@]} + 1 ))"
      if zle -A "${saved_widget}" "${saved_alias}" 2>/dev/null; then
        __qtpi_saved_widget_aliases[$binding_id]="${saved_alias}"
      else
        __qtpi_saved_widget_aliases[$binding_id]=''
      fi
    fi
  fi

  if [[ -z "${__qtpi_saved_widgets_by_widget[$widget_binding_id]+set}" ]]; then
    __qtpi_saved_widgets_by_widget[$widget_binding_id]="${__qtpi_saved_bindings[$binding_id]-}"
  fi

  if [[ -z "${__qtpi_saved_widget_aliases_by_widget[$widget_binding_id]+set}" ]]; then
    __qtpi_saved_widget_aliases_by_widget[$widget_binding_id]="${__qtpi_saved_widget_aliases[$binding_id]-}"
  fi

  __qtpi_managed_bindings[$binding_id]="${widget}"
  bindkey -M "${keymap}" "${key}" "${widget}"
}

function __qtpi_restore_binding_in_keymap() {
  local keymap="$1"
  local key="$2"
  local binding_id="$(__qtpi_binding_id "${keymap}" "${key}")"

  if [[ -z "${__qtpi_managed_bindings[$binding_id]+set}" ]]; then
    return 0
  fi

  local widget="${__qtpi_managed_bindings[$binding_id]}"
  local widget_binding_id="$(__qtpi_binding_id "${keymap}" "${widget}")"
  local saved="${__qtpi_saved_bindings[$binding_id]-}"
  local current="$(__qtpi_widget_for_key "${key}" "${keymap}")"

  saved="$(__qtpi_resolve_original_widget "${keymap}" "${widget}" "${saved}")"

  if [[ "${current}" == "${widget}" ]]; then
    if [[ -z "${saved}" || "${saved}" == 'undefined-key' ]]; then
      bindkey -M "${keymap}" -r "${key}" 2>/dev/null
    else
      bindkey -M "${keymap}" "${key}" "${saved}"
    fi
  fi

  unset "__qtpi_saved_bindings[$binding_id]"
  unset "__qtpi_managed_bindings[$binding_id]"
  unset "__qtpi_saved_widget_aliases[$binding_id]"
  unset "__qtpi_saved_widgets_by_widget[$widget_binding_id]"
  unset "__qtpi_saved_widget_aliases_by_widget[$widget_binding_id]"
}

function __qtpi_bind_widget() {
  local key="$1"
  local widget="$2"

  if [[ -z "${key}" ]]; then
    return 0
  fi

  local keymap
  for keymap in main emacs viins vicmd; do
    if __qtpi_keymap_exists "${keymap}"; then
      __qtpi_bind_widget_in_keymap "${keymap}" "${key}" "${widget}"
    fi
  done
}

function __qtpi_restore_binding() {
  local key="$1"

  if [[ -z "${key}" ]]; then
    return 0
  fi

  local keymap
  for keymap in main emacs viins vicmd; do
    if __qtpi_keymap_exists "${keymap}"; then
      __qtpi_restore_binding_in_keymap "${keymap}" "${key}"
    fi
  done
}

function qtpi_zsh_enable() {
  if (( __qtpi_enabled )); then
    return 0
  fi

  zle -N qtpi-show-or-refresh qtpi_show_or_refresh
  zle -N qtpi-accept-suggestion qtpi_accept_suggestion
  zle -N qtpi-accept-or-original qtpi_accept_or_original
  zle -N qtpi-dismiss-suggestions qtpi_dismiss_suggestions
  zle -N qtpi-interrupt-or-original qtpi_interrupt_or_original
  zle -N qtpi-next-suggestion qtpi_next_suggestion
  zle -N qtpi-previous-suggestion qtpi_previous_suggestion
  zle -N qtpi-down-or-original qtpi_down_or_original
  zle -N qtpi-up-or-original qtpi_up_or_original
  zle -N qtpi-space-maybe-show qtpi_space_maybe_show
  zle -N qtpi-backspace-or-original qtpi_backspace_or_original
  zle -N qtpi-self-insert qtpi_self_insert
  zle -N __qtpi-after-widget-ready __qtpi_after_widget_ready
  zle -N __qtpi-line-pre-redraw-cleanup __qtpi_line_pre_redraw_cleanup
  zle -N __qtpi-line-finish-cleanup __qtpi_line_finish_cleanup

  __qtpi_bind_widget "${QTPI_KEY_SHOW}" qtpi-show-or-refresh
  __qtpi_bind_widget ' ' qtpi-space-maybe-show

  add-zsh-hook precmd __qtpi_precmd_clear
  add-zsh-hook preexec __qtpi_precmd_clear
  add-zle-hook-widget line-pre-redraw __qtpi-line-pre-redraw-cleanup
  add-zle-hook-widget line-finish __qtpi-line-finish-cleanup

  __qtpi_enabled=1
}

function qtpi_zsh_disable() {
  if (( ! __qtpi_enabled )); then
    return 0
  fi

  __qtpi_restore_binding "${QTPI_KEY_SHOW}"
  __qtpi_restore_binding ' '

  add-zsh-hook -d precmd __qtpi_precmd_clear 2>/dev/null
  add-zsh-hook -d preexec __qtpi_precmd_clear 2>/dev/null
  add-zle-hook-widget -d line-pre-redraw __qtpi-line-pre-redraw-cleanup 2>/dev/null
  add-zle-hook-widget -d line-finish __qtpi-line-finish-cleanup 2>/dev/null

  __qtpi_deactivate_transient_bindings
  __qtpi_invalidate_menu
  __qtpi_enabled=0
}

if [[ -o interactive ]]; then
  qtpi_zsh_enable
fi

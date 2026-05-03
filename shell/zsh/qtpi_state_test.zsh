#!/usr/bin/env zsh

emulate -L zsh
setopt pipefail

SCRIPT_DIR="${0:A:h}"
source "${SCRIPT_DIR}/qtpi.zsh"

function __qtpi_activate_transient_bindings() {
  __qtpi_transient_bindings_active=1
}

function __qtpi_deactivate_transient_bindings() {
  __qtpi_transient_bindings_active=0
}

function __qtpi_disable_autosuggestions() {
  __qtpi_autosuggest_suppressed=1
}

function __qtpi_enable_autosuggestions() {
  __qtpi_autosuggest_suppressed=0
}

function __qtpi_overlay_capable() {
  __qtpi_overlay_supported=1
  return 0
}

function __qtpi_clear_overlay() {
  __qtpi_overlay_origin_row=0
  __qtpi_rendered_row_count=0
  __qtpi_skip_next_pre_redraw_clear=0
}

function __qtpi_render_overlay() {
  if ! __qtpi_menu_state_valid; then
    return 1
  fi

  __qtpi_overlay_window_end_index "${#__qtpi_insert_texts[@]}"
  __qtpi_rendered_row_count=$(( REPLY - __qtpi_overlay_window_start ))
  __qtpi_overlay_origin_row=1
  __qtpi_skip_next_pre_redraw_clear=1
  return 0
}

function fail() {
  print -u2 -- "FAIL: $1"
  exit 1
}

function assert_equal() {
  local actual="$1"
  local expected="$2"
  local label="$3"

  if [[ "${actual}" != "${expected}" ]]; then
    fail "${label}: expected '${expected}', got '${actual}'"
  fi
}

function assert_menu_state() {
  local visible="$1"
  local selection="$2"
  local window_start="$3"
  local label="$4"

  assert_equal "${__qtpi_menu_visible}" "${visible}" "${label} visible"
  assert_equal "${__qtpi_selection_index}" "${selection}" "${label} selection"
  assert_equal "${__qtpi_overlay_window_start}" "${window_start}" "${label} window_start"
}

function assert_true() {
  if ! "$@"; then
    fail "expected success: $*"
  fi
}

function assert_false() {
  if "$@"; then
    fail "expected failure: $*"
  fi
}

function seed_rows() {
  __qtpi_insert_texts=()
  __qtpi_displays=()
  __qtpi_annotations=()
  __qtpi_kinds=()

  local item
  for item in "$@"; do
    __qtpi_insert_texts+=("${item} ")
    __qtpi_displays+=("${item}")
    __qtpi_annotations+=('')
    __qtpi_kinds+=('subcommand')
  done
}

function __qtpi_query_suggestions() {
  __qtpi_status='ok'
  __qtpi_provider_id='git'
  __qtpi_parser_status='ok'
  __qtpi_request_buffer="${BUFFER}"
  __qtpi_request_cursor=${CURSOR}
  __qtpi_dynamic_slot_id=''
  __qtpi_lookup_status='not_checked'
  __qtpi_cache_status='not_checked'
  __qtpi_truncated_count=0
  __qtpi_lookup_count=0
  __qtpi_lookup_time_ms=0

  case "${BUFFER}" in
    'git ')
      __qtpi_replace_start=${CURSOR}
      __qtpi_replace_end=${CURSOR}
      seed_rows add branch checkout clone commit cherry-pick diff
      ;;
    'git c')
      __qtpi_replace_start=4
      __qtpi_replace_end=${CURSOR}
      seed_rows checkout clone commit cherry-pick
      ;;
    'git co')
      __qtpi_replace_start=4
      __qtpi_replace_end=${CURSOR}
      seed_rows checkout commit
      ;;
    'git com')
      __qtpi_replace_start=4
      __qtpi_replace_end=${CURSOR}
      seed_rows commit
      ;;
    'git cx')
      __qtpi_replace_start=4
      __qtpi_replace_end=${CURSOR}
      seed_rows
      ;;
    *)
      __qtpi_replace_start=${CURSOR}
      __qtpi_replace_end=${CURSOR}
      seed_rows
      ;;
  esac

  return 0
}

function reset_test_state() {
  BUFFER=''
  CURSOR=0
  __qtpi_after_widget_fd=-1
  __qtpi_after_widget_pid=''
  __qtpi_reset_state
}

function test_initial_show_and_typed_refresh() {
  reset_test_state

  BUFFER='git '
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_menu_state 1 0 0 'git space'
  assert_equal "${__qtpi_rendered_row_count}" '5' 'git space rendered rows'

  BUFFER='git c'
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_menu_state 1 0 0 'git c'

  BUFFER='git co'
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_menu_state 1 0 0 'git co'

  BUFFER='git com'
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_menu_state 1 0 0 'git com'

  BUFFER='git cx'
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_menu_state 0 -1 0 'git cx'
  assert_equal "${__qtpi_rendered_row_count}" '0' 'git cx rendered rows'
  assert_equal "${__qtpi_overlay_origin_row}" '0' 'git cx overlay origin'
}

function test_scroll_down_and_clamp_bottom() {
  reset_test_state
  seed_rows one two three four five six seven
  __qtpi_begin_menu_refresh || fail 'expected seeded rows to show'

  repeat 4; do
    __qtpi_move_selection_down || fail 'expected move down to succeed within list'
  done
  assert_menu_state 1 4 0 'selection at bottom of first frame'

  __qtpi_move_selection_down || fail 'expected move down across frame edge'
  assert_menu_state 1 5 1 'selection after first scroll down'

  __qtpi_move_selection_down || fail 'expected move down to list bottom'
  assert_menu_state 1 6 2 'selection at list bottom'

  __qtpi_move_selection_down || fail 'expected clamp move down to remain valid'
  assert_menu_state 1 6 2 'selection clamped at bottom'
}

function test_scroll_up_from_bottom() {
  reset_test_state
  seed_rows one two three four five six seven
  __qtpi_begin_menu_refresh || fail 'expected seeded rows to show'

  repeat 6; do
    __qtpi_move_selection_down || fail 'expected move down to succeed'
  done
  assert_menu_state 1 6 2 'setup bottom state'

  __qtpi_move_selection_up || fail 'expected first move up from bottom'
  assert_menu_state 1 5 2 'first move up stays in frame'

  __qtpi_move_selection_up || fail 'expected second move up from bottom'
  assert_menu_state 1 4 2 'second move up stays in frame'

  __qtpi_move_selection_up || fail 'expected third move up from bottom'
  assert_menu_state 1 3 2 'third move up stays in frame'

  __qtpi_move_selection_up || fail 'expected fourth move up from bottom'
  assert_menu_state 1 2 2 'fourth move up stays at frame edge'

  __qtpi_move_selection_up || fail 'expected frame to scroll upward'
  assert_menu_state 1 1 1 'frame scrolls only after leaving top edge'

  __qtpi_move_selection_up || fail 'expected move to top'
  assert_menu_state 1 0 0 'frame returns to top'

  __qtpi_move_selection_up || fail 'expected clamp at top'
  assert_menu_state 1 0 0 'selection clamped at top'
}

function test_backspace_refresh_resets_selection() {
  reset_test_state

  BUFFER='git '
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  repeat 3; do
    __qtpi_move_selection_down || fail 'expected move down before typed narrowing'
  done
  assert_menu_state 1 3 0 'pre-refresh selection'

  BUFFER='git co'
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_menu_state 1 0 0 'typed narrowing resets selection'

  __qtpi_move_selection_down || fail 'expected move down within narrowed list'
  assert_menu_state 1 1 0 'narrowed selection moved'

  BUFFER='git c'
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_menu_state 1 0 0 'backspace refresh resets selection'
}

function test_enter_accept_requires_fragment_or_explicit_selection() {
  reset_test_state

  BUFFER='git '
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_false __qtpi_enter_should_accept_selection

  __qtpi_move_selection_down || fail 'expected explicit selection move'
  assert_true __qtpi_enter_should_accept_selection

  BUFFER='git c'
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_true __qtpi_enter_should_accept_selection
}

function test_invalidation_resets_overlay_state() {
  reset_test_state

  BUFFER='git '
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_equal "${__qtpi_rendered_row_count}" '5' 'pre-invalidation rendered rows'

  __qtpi_overlay_highlight_supported=1
  __qtpi_overlay_highlight_enter='['
  __qtpi_overlay_highlight_exit=']'

  BUFFER='git cx'
  CURSOR=${#BUFFER}
  __qtpi_refresh_now
  assert_menu_state 0 -1 0 'no-match invalidation'
  assert_equal "${__qtpi_rendered_row_count}" '0' 'no-match rendered rows'
  assert_equal "${__qtpi_overlay_origin_row}" '0' 'no-match origin row'
  assert_equal "${__qtpi_skip_next_pre_redraw_clear}" '0' 'no-match skip redraw flag'
  assert_equal "${__qtpi_overlay_highlight_supported}" '0' 'no-match highlight support'
  assert_equal "${__qtpi_overlay_highlight_enter}" '' 'no-match highlight enter'
  assert_equal "${__qtpi_overlay_highlight_exit}" '' 'no-match highlight exit'
}

function test_missing_binary_degrades_cleanly() {
  reset_test_state

  QTPI_BIN='/definitely/missing/qtpi'
  assert_false __qtpi_runtime_available
  QTPI_BIN='qtpi'
}

test_initial_show_and_typed_refresh
test_scroll_down_and_clamp_bottom
test_scroll_up_from_bottom
test_backspace_refresh_resets_selection
test_enter_accept_requires_fragment_or_explicit_selection
test_invalidation_resets_overlay_state
test_missing_binary_degrades_cleanly

print -- 'qtpi zsh bridge state tests passed'

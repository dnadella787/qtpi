#!/usr/bin/env zsh

emulate -L zsh
setopt pipefail

SCRIPT_DIR="${0:A:h}"
source "${SCRIPT_DIR}/twocp.zsh"

function __twocp_activate_transient_bindings() {
  __twocp_transient_bindings_active=1
}

function __twocp_deactivate_transient_bindings() {
  __twocp_transient_bindings_active=0
}

function __twocp_disable_autosuggestions() {
  __twocp_autosuggest_suppressed=1
}

function __twocp_enable_autosuggestions() {
  __twocp_autosuggest_suppressed=0
}

function __twocp_overlay_capable() {
  __twocp_overlay_supported=1
  return 0
}

function __twocp_clear_overlay() {
  __twocp_overlay_origin_row=0
  __twocp_rendered_row_count=0
  __twocp_skip_next_pre_redraw_clear=0
}

function __twocp_render_overlay() {
  if ! __twocp_menu_state_valid; then
    return 1
  fi

  __twocp_overlay_window_end_index "${#__twocp_insert_texts[@]}"
  __twocp_rendered_row_count=$(( REPLY - __twocp_overlay_window_start ))
  __twocp_overlay_origin_row=1
  __twocp_skip_next_pre_redraw_clear=1
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

  assert_equal "${__twocp_menu_visible}" "${visible}" "${label} visible"
  assert_equal "${__twocp_selection_index}" "${selection}" "${label} selection"
  assert_equal "${__twocp_overlay_window_start}" "${window_start}" "${label} window_start"
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
  __twocp_insert_texts=()
  __twocp_displays=()
  __twocp_annotations=()
  __twocp_kinds=()

  local item
  for item in "$@"; do
    __twocp_insert_texts+=("${item} ")
    __twocp_displays+=("${item}")
    __twocp_annotations+=('')
    __twocp_kinds+=('subcommand')
  done
}

function __twocp_query_suggestions() {
  __twocp_status='ok'
  __twocp_provider_id='git'
  __twocp_parser_status='ok'
  __twocp_request_buffer="${BUFFER}"
  __twocp_request_cursor=${CURSOR}
  __twocp_dynamic_slot_id=''
  __twocp_lookup_status='not_checked'
  __twocp_cache_status='not_checked'
  __twocp_truncated_count=0
  __twocp_lookup_count=0
  __twocp_lookup_time_ms=0

  case "${BUFFER}" in
    'git ')
      __twocp_replace_start=${CURSOR}
      __twocp_replace_end=${CURSOR}
      seed_rows add branch checkout clone commit cherry-pick diff
      ;;
    'git c')
      __twocp_replace_start=4
      __twocp_replace_end=${CURSOR}
      seed_rows checkout clone commit cherry-pick
      ;;
    'git co')
      __twocp_replace_start=4
      __twocp_replace_end=${CURSOR}
      seed_rows checkout commit
      ;;
    'git com')
      __twocp_replace_start=4
      __twocp_replace_end=${CURSOR}
      seed_rows commit
      ;;
    'git cx')
      __twocp_replace_start=4
      __twocp_replace_end=${CURSOR}
      seed_rows
      ;;
    *)
      __twocp_replace_start=${CURSOR}
      __twocp_replace_end=${CURSOR}
      seed_rows
      ;;
  esac

  return 0
}

function reset_test_state() {
  BUFFER=''
  CURSOR=0
  __twocp_after_widget_fd=-1
  __twocp_after_widget_pid=''
  __twocp_reset_state
}

function test_initial_show_and_typed_refresh() {
  reset_test_state

  BUFFER='git '
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_menu_state 1 0 0 'git space'
  assert_equal "${__twocp_rendered_row_count}" '5' 'git space rendered rows'

  BUFFER='git c'
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_menu_state 1 0 0 'git c'

  BUFFER='git co'
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_menu_state 1 0 0 'git co'

  BUFFER='git com'
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_menu_state 1 0 0 'git com'

  BUFFER='git cx'
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_menu_state 0 -1 0 'git cx'
  assert_equal "${__twocp_rendered_row_count}" '0' 'git cx rendered rows'
  assert_equal "${__twocp_overlay_origin_row}" '0' 'git cx overlay origin'
}

function test_scroll_down_and_clamp_bottom() {
  reset_test_state
  seed_rows one two three four five six seven
  __twocp_begin_menu_refresh || fail 'expected seeded rows to show'

  repeat 4; do
    __twocp_move_selection_down || fail 'expected move down to succeed within list'
  done
  assert_menu_state 1 4 0 'selection at bottom of first frame'

  __twocp_move_selection_down || fail 'expected move down across frame edge'
  assert_menu_state 1 5 1 'selection after first scroll down'

  __twocp_move_selection_down || fail 'expected move down to list bottom'
  assert_menu_state 1 6 2 'selection at list bottom'

  __twocp_move_selection_down || fail 'expected clamp move down to remain valid'
  assert_menu_state 1 6 2 'selection clamped at bottom'
}

function test_scroll_up_from_bottom() {
  reset_test_state
  seed_rows one two three four five six seven
  __twocp_begin_menu_refresh || fail 'expected seeded rows to show'

  repeat 6; do
    __twocp_move_selection_down || fail 'expected move down to succeed'
  done
  assert_menu_state 1 6 2 'setup bottom state'

  __twocp_move_selection_up || fail 'expected first move up from bottom'
  assert_menu_state 1 5 2 'first move up stays in frame'

  __twocp_move_selection_up || fail 'expected second move up from bottom'
  assert_menu_state 1 4 2 'second move up stays in frame'

  __twocp_move_selection_up || fail 'expected third move up from bottom'
  assert_menu_state 1 3 2 'third move up stays in frame'

  __twocp_move_selection_up || fail 'expected fourth move up from bottom'
  assert_menu_state 1 2 2 'fourth move up stays at frame edge'

  __twocp_move_selection_up || fail 'expected frame to scroll upward'
  assert_menu_state 1 1 1 'frame scrolls only after leaving top edge'

  __twocp_move_selection_up || fail 'expected move to top'
  assert_menu_state 1 0 0 'frame returns to top'

  __twocp_move_selection_up || fail 'expected clamp at top'
  assert_menu_state 1 0 0 'selection clamped at top'
}

function test_backspace_refresh_resets_selection() {
  reset_test_state

  BUFFER='git '
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  repeat 3; do
    __twocp_move_selection_down || fail 'expected move down before typed narrowing'
  done
  assert_menu_state 1 3 0 'pre-refresh selection'

  BUFFER='git co'
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_menu_state 1 0 0 'typed narrowing resets selection'

  __twocp_move_selection_down || fail 'expected move down within narrowed list'
  assert_menu_state 1 1 0 'narrowed selection moved'

  BUFFER='git c'
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_menu_state 1 0 0 'backspace refresh resets selection'
}

function test_enter_accept_requires_fragment_or_explicit_selection() {
  reset_test_state

  BUFFER='git '
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_false __twocp_enter_should_accept_selection

  __twocp_move_selection_down || fail 'expected explicit selection move'
  assert_true __twocp_enter_should_accept_selection

  BUFFER='git c'
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_true __twocp_enter_should_accept_selection
}

function test_invalidation_resets_overlay_state() {
  reset_test_state

  BUFFER='git '
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_equal "${__twocp_rendered_row_count}" '5' 'pre-invalidation rendered rows'

  __twocp_overlay_highlight_supported=1
  __twocp_overlay_highlight_enter='['
  __twocp_overlay_highlight_exit=']'

  BUFFER='git cx'
  CURSOR=${#BUFFER}
  __twocp_refresh_now
  assert_menu_state 0 -1 0 'no-match invalidation'
  assert_equal "${__twocp_rendered_row_count}" '0' 'no-match rendered rows'
  assert_equal "${__twocp_overlay_origin_row}" '0' 'no-match origin row'
  assert_equal "${__twocp_skip_next_pre_redraw_clear}" '0' 'no-match skip redraw flag'
  assert_equal "${__twocp_overlay_highlight_supported}" '0' 'no-match highlight support'
  assert_equal "${__twocp_overlay_highlight_enter}" '' 'no-match highlight enter'
  assert_equal "${__twocp_overlay_highlight_exit}" '' 'no-match highlight exit'
}

test_initial_show_and_typed_refresh
test_scroll_down_and_clamp_bottom
test_scroll_up_from_bottom
test_backspace_refresh_resets_selection
test_enter_accept_requires_fragment_or_explicit_selection
test_invalidation_resets_overlay_state

print -- 'twocp zsh bridge state tests passed'

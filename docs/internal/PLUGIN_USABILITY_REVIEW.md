# Plugin Usability Review

Review of plugin usability for: Find References, Live Grep, Git Grep, Git Find File, Search Replace in Project, and Path Complete.

## Summary

Testing revealed two critical infrastructure bugs affecting multiple plugins, plus several plugin-specific usability issues. The core problems are in the plugin mode keybinding system and i18n template substitution.

---

## Critical (P0) - Core Plugin Infrastructure Bugs

### 1. Custom mode keybindings don't work

**Affected Plugins:** Find References, Search Replace

**Details:** Keys defined via `editor.defineMode()` are completely non-functional:
- `q`, `Escape` (close panel)
- `Enter`/`Return` (activate/jump)
- `space`, `a`, `n`, `r` (Search Replace actions)

Navigation with arrow keys works (cursor moves), but all action keys fail. Users cannot close panels or activate items using the documented keybindings.

**nngroup Violation:** User control and freedom - users are trapped in panels with no way to exit via keyboard.

**Expected behavior:** All keybindings defined in `editor.defineMode()` should work when the virtual buffer has focus.

### 2. Template variable substitution broken in `editor.t()`

**Affected Plugins:** Find References, Search Replace

**Details:** The i18n system returns template strings without interpolating parameters:
- Shows `{symbol}`, `{count}`, `{limit}` instead of actual values
- Example: "References to {symbol} ({count}{limit})" instead of "References to 'Args' (4)"

**nngroup Violation:** Visibility of system status - users cannot see actual counts or context.

**Expected behavior:** `editor.t("key", { param: value })` should substitute `{param}` with `value`.

---

## High (P1) - Plugin-Specific Bugs

### 3. Live Grep: Preview doesn't update on navigation

**Details:** When pressing Up/Down to navigate results, the preview pane stays on the first result instead of updating to show the currently selected item.

**nngroup Violation:** Visibility of system status

### 4. Live Grep: Selection mismatch on confirm

**Details:** The file that opens on Enter is different from what's shown in the preview. For example, preview shows `build.rs:714` but `types/fresh.d.ts.template` opens.

**nngroup Violation:** Consistency and standards - unpredictable behavior

### 5. Git Grep: Opens file at wrong position

**Details:** Files open at line 1 instead of the matched line location, defeating the purpose of the search.

**nngroup Violation:** Match between system and real world

### 6. All search plugins: No visual selection indicator

**Affected Plugins:** Live Grep, Git Grep, Git Find File, Find References, Search Replace

**Details:** When navigating results with arrow keys, there's no visual indication of which item is currently selected. Users navigate blind.

**nngroup Violation:** Visibility of system status

---

## Medium (P2) - UX Improvements

### 7. No keyboard shortcuts for plugin commands

**Affected:** Live Grep, Git Grep, Git Find File, Search Replace

**Details:** These frequently-used commands have no keyboard shortcuts assigned. Users must open command palette every time.

**nngroup Violation:** Flexibility and efficiency of use

### 8. Inconsistent features across search plugins

**Details:** Live Grep has a preview pane; Git Grep doesn't. This inconsistency can confuse users switching between similar tools.

**nngroup Violation:** Consistency and standards

### 9. Status messages truncated

**Details:** Important error messages are cut off in the status bar (e.g., "Failed to op..."). Users cannot see full error details.

**nngroup Violation:** Help users recognize, diagnose, and recover from errors

---

## Low (P3) - Minor Issues

### 10. Git Find File: "New file" suggestion not shown

**Details:** When no files match the search, the "Create new file" option mentioned in code isn't visible to users.

### 11. Tab key behavior in Live Grep

**Details:** Tab replaces search text instead of being ignored or completing, causing accidental input loss.

---

## Positive Observations

- **Path Complete / File Browser:** Works well with live filtering and path navigation
- **Help text in panels:** Find References and Search Replace show keybinding help (though keybindings don't work)
- **Git Find File fuzzy search:** Good algorithm with intelligent scoring
- **Live Grep split preview concept:** Good design when working correctly
- **Arrow key navigation:** Works correctly across all plugins for moving cursor

---

## Design Principle

**Plugins should prioritize normal keys (arrow keys, Enter) for selecting and activating items.**

This is partially implemented - arrow keys work for navigation. However, Enter doesn't work due to the mode keybinding bug (P0 #1).

---

## Recommended Fix Priority

1. **Fix `editor.defineMode()` keybinding activation** - Unblocks all panel interactions
2. **Fix `editor.t()` parameter substitution** - Restores status visibility
3. **Add visual selection indicator** - Fundamental UX requirement
4. **Fix Live Grep preview update** - Core feature broken
5. **Fix Git Grep cursor positioning** - Core feature broken

---

## Files to Investigate

- `src/services/plugins/` - Plugin runtime, mode registration
- `src/app/plugin_commands.rs` - Command handlers for plugin API
- `src/i18n.rs` or plugin i18n handling - Template substitution logic
- `plugins/live_grep.ts:331-345` - `onLiveGrepSelectionChanged` handler
- `plugins/git_grep.ts` - File opening logic

/// <reference path="../types/fresh.d.ts" />

/**
 * Buffer Modified Plugin
 *
 * Shows indicators in the gutter for lines that have been modified since the last save.
 * This tracks in-memory changes, not git changes.
 *
 * This plugin uses a simpler approach: it marks lines as modified when edits happen
 * (after_insert/after_delete hooks), and clears all modified markers on save.
 * It doesn't compare content - it just tracks which lines have been touched since save.
 *
 * Indicator symbols:
 * - │ (blue): Line has been modified since last save
 */

// =============================================================================
// Constants
// =============================================================================

const NAMESPACE = "buffer-modified";
const PRIORITY = 5; // Lower than git-gutter (10) and diagnostics

// Colors (RGB) - Blue to distinguish from git gutter (green/yellow/red)
const COLOR = [100, 149, 237] as [number, number, number]; // Cornflower blue

// Symbol
const SYMBOL = "│";

// =============================================================================
// Types
// =============================================================================

interface BufferState {
  /** Set of line numbers (0-indexed) that have been modified since last save */
  modifiedLines: Set<number>;
  /** Whether we're tracking this buffer */
  tracking: boolean;
}

// =============================================================================
// State
// =============================================================================

/** State per buffer */
const bufferStates: Map<number, BufferState> = new Map();

// =============================================================================
// Line Tracking
// =============================================================================

/**
 * Initialize state for a buffer (on file open)
 * Starts with no modified lines since file was just loaded
 */
function initBufferState(bufferId: number): void {
  bufferStates.set(bufferId, {
    modifiedLines: new Set(),
    tracking: true,
  });
  // Clear any leftover indicators
  editor.clearLineIndicators(bufferId, NAMESPACE);
}

/**
 * Clear modified state for a buffer (on save)
 * Removes all modified markers since buffer now matches disk
 */
function clearModifiedState(bufferId: number): void {
  const state = bufferStates.get(bufferId);
  if (state) {
    state.modifiedLines.clear();
  }
  editor.clearLineIndicators(bufferId, NAMESPACE);
}

/**
 * Mark a range of lines as modified and update indicators
 */
function markLinesModified(bufferId: number, startLine: number, endLine: number): void {
  const state = bufferStates.get(bufferId);
  if (!state || !state.tracking) return;

  // Mark all lines in range as modified
  for (let line = startLine; line <= endLine; line++) {
    if (!state.modifiedLines.has(line)) {
      state.modifiedLines.add(line);
      // Add indicator for this line
      editor.setLineIndicator(
        bufferId,
        line,
        NAMESPACE,
        SYMBOL,
        COLOR[0],
        COLOR[1],
        COLOR[2],
        PRIORITY
      );
    }
  }
}

// =============================================================================
// Event Handlers
// =============================================================================

/**
 * Handle after file open - initialize state
 */
globalThis.onBufferModifiedAfterFileOpen = function (args: {
  buffer_id: number;
  path: string;
}): boolean {
  const bufferId = args.buffer_id;

  if (!args.path || args.path === "") {
    return true;
  }

  // Initialize tracking - file just loaded, no modifications yet
  initBufferState(bufferId);
  editor.debug(`Buffer Modified: initialized for ${args.path}`);

  return true;
};

/**
 * Handle buffer activation - ensure we're tracking
 */
globalThis.onBufferModifiedBufferActivated = function (args: {
  buffer_id: number;
}): boolean {
  const bufferId = args.buffer_id;

  // If we don't have state yet, initialize it
  if (!bufferStates.has(bufferId)) {
    const filePath = editor.getBufferPath(bufferId);
    if (filePath && filePath !== "") {
      initBufferState(bufferId);
    }
  }

  return true;
};

/**
 * Handle after file save - clear modified state
 */
globalThis.onBufferModifiedAfterSave = function (args: {
  buffer_id: number;
  path: string;
}): boolean {
  const bufferId = args.buffer_id;

  // Clear all modified markers - buffer now matches disk
  clearModifiedState(bufferId);
  editor.debug("Buffer Modified: cleared on save");

  return true;
};

/**
 * Handle after insert - mark affected lines as modified
 */
globalThis.onBufferModifiedAfterInsert = function (args: {
  buffer_id: number;
  position: number;
  text: string;
  affected_start: number;
  affected_end: number;
  start_line: number;
  end_line: number;
  lines_added: number;
}): boolean {
  const bufferId = args.buffer_id;

  if (!bufferStates.has(bufferId)) {
    return true;
  }

  const state = bufferStates.get(bufferId)!;

  // First, shift existing indicators if lines were added
  if (args.lines_added > 0) {
    const shiftedLines = new Set<number>();
    for (const line of state.modifiedLines) {
      if (line >= args.start_line) {
        // Shift lines at or after insertion point
        shiftedLines.add(line + args.lines_added);
      } else {
        shiftedLines.add(line);
      }
    }
    state.modifiedLines = shiftedLines;
  }

  // Mark all affected lines (from start_line to end_line inclusive)
  markLinesModified(bufferId, args.start_line, args.end_line);

  return true;
};

/**
 * Handle after delete - mark affected lines as modified
 */
globalThis.onBufferModifiedAfterDelete = function (args: {
  buffer_id: number;
  start: number;
  end: number;
  deleted_text: string;
  affected_start: number;
  deleted_len: number;
  start_line: number;
  end_line: number;
  lines_removed: number;
}): boolean {
  const bufferId = args.buffer_id;

  if (!bufferStates.has(bufferId)) {
    return true;
  }

  const state = bufferStates.get(bufferId)!;

  // Shift existing indicators if lines were removed
  if (args.lines_removed > 0) {
    const shiftedLines = new Set<number>();
    for (const line of state.modifiedLines) {
      if (line > args.end_line) {
        // Lines after the deleted range shift up
        shiftedLines.add(line - args.lines_removed);
      } else if (line < args.start_line) {
        // Lines before deletion are unchanged
        shiftedLines.add(line);
      }
      // Lines within the deleted range are removed (not added to shiftedLines)
    }
    state.modifiedLines = shiftedLines;
  }

  // Mark the line where deletion occurred
  markLinesModified(bufferId, args.start_line, args.start_line);

  return true;
};

/**
 * Handle buffer closed - cleanup state
 */
globalThis.onBufferModifiedBufferClosed = function (args: {
  buffer_id: number;
}): boolean {
  bufferStates.delete(args.buffer_id);
  return true;
};

// =============================================================================
// Registration
// =============================================================================

// Register event handlers
editor.on("after_file_open", "onBufferModifiedAfterFileOpen");
editor.on("buffer_activated", "onBufferModifiedBufferActivated");
editor.on("after_file_save", "onBufferModifiedAfterSave");
editor.on("after-insert", "onBufferModifiedAfterInsert");
editor.on("after-delete", "onBufferModifiedAfterDelete");
editor.on("buffer_closed", "onBufferModifiedBufferClosed");

// Initialize for the current buffer
const initBufferId = editor.getActiveBufferId();
const initPath = editor.getBufferPath(initBufferId);
if (initPath && initPath !== "") {
  initBufferState(initBufferId);
}

editor.debug("Buffer Modified plugin loaded");

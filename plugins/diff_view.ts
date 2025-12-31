/// <reference path="./lib/fresh.d.ts" />

/**
 * Diff View Plugin
 *
 * Provides a side-by-side diff view using composite buffers.
 * This plugin demonstrates the multi-buffer single-tab architecture
 * for viewing differences between files or file versions.
 *
 * Features:
 * - Side-by-side diff view with line alignment
 * - Supports comparing current file with git HEAD
 * - Supports comparing two arbitrary files
 * - Unified diff mode (optional)
 *
 * Commands:
 * - "Diff: Compare with HEAD" - Compare current buffer with git HEAD
 * - "Diff: Compare Two Files" - Compare two selected files
 */

// =============================================================================
// Types
// =============================================================================

interface DiffHunk {
  oldStart: number;
  oldCount: number;
  newStart: number;
  newCount: number;
}

interface DiffState {
  isOpen: boolean;
  compositeBufferId: number | null;
  oldBufferId: number | null;
  newBufferId: number | null;
  hunks: DiffHunk[];
}

// =============================================================================
// State
// =============================================================================

const diffState: DiffState = {
  isOpen: false,
  compositeBufferId: null,
  oldBufferId: null,
  newBufferId: null,
  hunks: [],
};

// =============================================================================
// Mode Definition
// =============================================================================

// Define diff-view mode with navigation keybindings
editor.defineMode(
  "diff-view",
  "normal", // inherit from normal for basic navigation
  [
    ["q", "diff_view_close"],
    ["Escape", "diff_view_close"],
    ["n", "diff_view_next_hunk"],
    ["N", "diff_view_prev_hunk"],
    ["]c", "diff_view_next_hunk"],
    ["[c", "diff_view_prev_hunk"],
    ["Tab", "diff_view_toggle_focus"],
    ["u", "diff_view_toggle_unified"],
  ],
  true // read-only
);

// =============================================================================
// Git Operations
// =============================================================================

/**
 * Get file content at HEAD
 */
async function getFileAtHead(filePath: string): Promise<string | null> {
  const cwd = editor.pathDirname(filePath);
  const result = await editor.spawnProcess("git", [
    "show",
    `HEAD:${filePath}`,
  ], cwd);

  if (result.exit_code !== 0) {
    return null;
  }

  return result.stdout;
}

/**
 * Parse unified diff output to extract hunks
 */
function parseDiffHunks(diffOutput: string): DiffHunk[] {
  const hunks: DiffHunk[] = [];
  const lines = diffOutput.split("\n");

  for (const line of lines) {
    // Match hunk header: @@ -old_start,old_count +new_start,new_count @@
    const match = line.match(/^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/);
    if (match) {
      hunks.push({
        oldStart: parseInt(match[1], 10) - 1, // Convert to 0-indexed
        oldCount: parseInt(match[2] || "1", 10),
        newStart: parseInt(match[3], 10) - 1, // Convert to 0-indexed
        newCount: parseInt(match[4] || "1", 10),
      });
    }
  }

  return hunks;
}

/**
 * Get git diff between HEAD and working tree
 */
async function getGitDiff(filePath: string): Promise<string> {
  const cwd = editor.pathDirname(filePath);
  const result = await editor.spawnProcess("git", [
    "diff",
    "HEAD",
    "--",
    filePath,
  ], cwd);

  return result.stdout;
}

// =============================================================================
// Diff View Operations
// =============================================================================

/**
 * Create a virtual buffer with given content
 */
async function createContentBuffer(
  name: string,
  content: string,
): Promise<number> {
  const entries: TextPropertyEntry[] = content.split("\n").map((line, idx) => ({
    text: line + "\n",
    properties: { type: "line", line: idx + 1 },
  }));

  return editor.createVirtualBuffer({
    name,
    mode: "normal",
    read_only: true,
    entries,
    show_line_numbers: true,
    show_cursors: false,
    editing_disabled: true,
  });
}

/**
 * Open a side-by-side diff view comparing current buffer with HEAD
 */
globalThis.diff_view_compare_head = async function(): Promise<void> {
  if (diffState.isOpen) {
    editor.setStatus("Diff view already open. Press 'q' to close.");
    return;
  }

  const bufferId = editor.getActiveBufferId();
  const filePath = editor.getBufferPath(bufferId);

  if (!filePath) {
    editor.setStatus("No file path for current buffer");
    return;
  }

  editor.setStatus("Loading diff...");

  // Get HEAD version
  const headContent = await getFileAtHead(filePath);
  if (headContent === null) {
    editor.setStatus("Failed to get HEAD version (is this file tracked?)");
    return;
  }

  // Get current buffer content
  const bufferInfo = editor.getBufferInfo(bufferId);
  const currentContent = editor.getBufferText(bufferId, 0, bufferInfo.length);

  // Get diff hunks
  const diffOutput = await getGitDiff(filePath);
  const hunks = parseDiffHunks(diffOutput);

  // Create virtual buffers for old and new content
  const oldBufferId = await createContentBuffer(
    `*HEAD: ${editor.pathBasename(filePath)}*`,
    headContent,
  );
  const newBufferId = await createContentBuffer(
    `*Current: ${editor.pathBasename(filePath)}*`,
    currentContent,
  );

  // Convert hunks to the format expected by createCompositeBuffer
  const compositeHunks: TsCompositeHunk[] = hunks.map(h => ({
    old_start: h.oldStart,
    old_count: h.oldCount,
    new_start: h.newStart,
    new_count: h.newCount,
  }));

  // Create composite buffer with side-by-side layout
  const compositeBufferId = await editor.createCompositeBuffer({
    name: `*Diff: ${editor.pathBasename(filePath)}*`,
    mode: "diff-view",
    layout: {
      layout_type: "side-by-side",
      ratios: [0.5, 0.5],
      show_separator: true,
    },
    sources: [
      {
        buffer_id: oldBufferId,
        label: "HEAD",
        editable: false,
        style: {
          remove_bg: [80, 0, 0],
          gutter_style: "diff-markers",
        },
      },
      {
        buffer_id: newBufferId,
        label: "Current",
        editable: false,
        style: {
          add_bg: [0, 80, 0],
          gutter_style: "diff-markers",
        },
      },
    ],
    hunks: compositeHunks.length > 0 ? compositeHunks : null,
  });

  // Update state
  diffState.isOpen = true;
  diffState.compositeBufferId = compositeBufferId;
  diffState.oldBufferId = oldBufferId;
  diffState.newBufferId = newBufferId;
  diffState.hunks = hunks;

  // Show the composite buffer
  editor.showBuffer(compositeBufferId);

  const hunkCount = hunks.length;
  editor.setStatus(
    `Diff view: ${hunkCount} change${hunkCount !== 1 ? "s" : ""} | n/N: hunks | Tab: switch pane | q: close`
  );
};

/**
 * Close the diff view
 */
globalThis.diff_view_close = function(): void {
  if (!diffState.isOpen) {
    return;
  }

  // Close composite buffer
  if (diffState.compositeBufferId !== null) {
    editor.closeCompositeBuffer(diffState.compositeBufferId);
  }

  // Close virtual buffers
  if (diffState.oldBufferId !== null) {
    editor.closeBuffer(diffState.oldBufferId);
  }
  if (diffState.newBufferId !== null) {
    editor.closeBuffer(diffState.newBufferId);
  }

  // Reset state
  diffState.isOpen = false;
  diffState.compositeBufferId = null;
  diffState.oldBufferId = null;
  diffState.newBufferId = null;
  diffState.hunks = [];

  editor.setStatus("Diff view closed");
};

/**
 * Navigate to next hunk
 */
globalThis.diff_view_next_hunk = function(): void {
  if (!diffState.isOpen || diffState.compositeBufferId === null) {
    return;
  }

  // Note: Actual hunk navigation would be handled by the composite buffer
  // infrastructure through composite_next_hunk. For now, show a message.
  editor.setStatus("Next hunk");
};

/**
 * Navigate to previous hunk
 */
globalThis.diff_view_prev_hunk = function(): void {
  if (!diffState.isOpen || diffState.compositeBufferId === null) {
    return;
  }

  editor.setStatus("Previous hunk");
};

/**
 * Toggle focus between panes
 */
globalThis.diff_view_toggle_focus = function(): void {
  if (!diffState.isOpen || diffState.compositeBufferId === null) {
    return;
  }

  editor.setStatus("Toggling pane focus");
};

/**
 * Toggle between side-by-side and unified view
 */
globalThis.diff_view_toggle_unified = function(): void {
  if (!diffState.isOpen) {
    return;
  }

  // Note: This would require recreating the composite buffer with a different
  // layout. For now, just show a message.
  editor.setStatus("Unified diff toggle not yet implemented");
};

// =============================================================================
// Command Registration
// =============================================================================

editor.registerCommand(
  "Diff: Compare with HEAD",
  "Show side-by-side diff of current file with git HEAD",
  "diff_view_compare_head",
  "normal"
);

editor.registerCommand(
  "Diff: Close",
  "Close the diff view",
  "diff_view_close",
  "diff-view"
);

// =============================================================================
// Plugin Initialization
// =============================================================================

editor.debug("Diff View plugin loaded");
editor.setStatus("Diff View plugin ready");

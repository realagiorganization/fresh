/// <reference path="./fresh.d.ts" />

import type { Location, RGB } from "./types.ts";

/**
 * ResultsPanel - High-level abstraction for displaying navigable lists
 *
 * This provides a Provider-pattern implementation where:
 * - Plugins provide data (items with labels and locations)
 * - The ResultsPanel handles UI (navigation, selection, highlighting)
 *
 * All result panels use consistent keybindings:
 * - Up/Down: Navigate items (handled by cursor movement in normal mode)
 * - Enter: Activate selected item (calls onSelect)
 * - Escape: Close panel (calls onClose)
 *
 * @example
 * ```typescript
 * const panel = new ResultsPanel(editor, "references", {
 *   onSelect: (item) => {
 *     editor.openFile(item.location.file, item.location.line, item.location.column);
 *   },
 *   onClose: () => {
 *     // cleanup
 *   },
 * });
 *
 * await panel.show({
 *   title: "References to 'foo'",
 *   items: references.map(ref => ({
 *     label: `${ref.file}:${ref.line}`,
 *     description: ref.lineText,
 *     location: ref,
 *   })),
 * });
 * ```
 */

/**
 * Item to display in a results panel
 */
export interface ResultItem {
  /** Primary text shown for this item */
  label: string;
  /** Secondary text (e.g., code preview) */
  description?: string;
  /** Location to jump to when selected */
  location?: Location;
  /** Custom data attached to this item */
  data?: unknown;
}

/**
 * Options for creating a ResultsPanel
 */
export interface ResultsPanelOptions {
  /** Called when user presses Enter on an item */
  onSelect?: (item: ResultItem, index: number) => void;
  /** Called when user presses Escape */
  onClose?: () => void;
  /** Called when cursor moves to a new item (for preview) */
  onCursorMove?: (item: ResultItem, index: number) => void;
  /** Split ratio (default 0.7 = main area keeps 70%) */
  ratio?: number;
}

/**
 * Options for showing results
 */
export interface ShowResultsOptions {
  /** Title shown at top of panel */
  title: string;
  /** Items to display */
  items: ResultItem[];
  /** Optional help text at bottom */
  helpText?: string;
}

/**
 * Colors used for highlighting
 */
const colors = {
  selected: [80, 80, 120] as RGB,
  location: [150, 255, 150] as RGB,
  help: [150, 150, 150] as RGB,
  title: [200, 200, 255] as RGB,
};

/**
 * Internal state for a ResultsPanel
 */
interface PanelState {
  isOpen: boolean;
  bufferId: number | null;
  splitId: number | null;
  sourceSplitId: number | null;
  cachedContent: string;
  cursorLine: number;
  items: ResultItem[];
  title: string;
}

/**
 * ResultsPanel class - manages a results list panel
 */
export class ResultsPanel {
  private state: PanelState = {
    isOpen: false,
    bufferId: null,
    splitId: null,
    sourceSplitId: null,
    cachedContent: "",
    cursorLine: 1,
    items: [],
    title: "",
  };

  private readonly modeName: string;
  private readonly panelName: string;
  private readonly namespace: string;
  private readonly handlerPrefix: string;

  /**
   * Create a new ResultsPanel
   *
   * @param editor - The editor API instance
   * @param id - Unique identifier for this panel (e.g., "references", "diagnostics")
   * @param options - Panel configuration
   */
  constructor(
    private readonly editor: EditorAPI,
    private readonly id: string,
    private readonly options: ResultsPanelOptions = {}
  ) {
    this.modeName = `${id}-results`;
    this.panelName = `*${id.charAt(0).toUpperCase() + id.slice(1)}*`;
    this.namespace = id;
    this.handlerPrefix = `_results_panel_${id}`;

    // Define mode with minimal keybindings
    // Navigation is handled by inheriting from "normal" mode
    editor.defineMode(
      this.modeName,
      "normal", // Inherit from normal for cursor movement
      [
        ["Return", `${this.handlerPrefix}_select`],
        ["Escape", `${this.handlerPrefix}_close`],
      ],
      true // read-only
    );

    // Register global handlers
    this.registerHandlers();
  }

  /**
   * Whether the panel is currently open
   */
  get isOpen(): boolean {
    return this.state.isOpen;
  }

  /**
   * The panel's buffer ID (null if not open)
   */
  get bufferId(): number | null {
    return this.state.bufferId;
  }

  /**
   * The source split ID (where user was before opening)
   */
  get sourceSplitId(): number | null {
    return this.state.sourceSplitId;
  }

  /**
   * Show the results panel with the given items
   */
  async show(showOptions: ShowResultsOptions): Promise<void> {
    const { title, items, helpText } = showOptions;

    // Save source context if not already open
    if (!this.state.isOpen) {
      this.state.sourceSplitId = this.editor.getActiveSplitId();
    }

    // Store items and title
    this.state.items = items;
    this.state.title = title;

    // Build entries
    const entries = this.buildEntries(title, items, helpText);
    this.state.cachedContent = entries.map(e => e.text).join("");
    this.state.cursorLine = 2; // Start on first item (after title)

    try {
      const result = await this.editor.createVirtualBufferInSplit({
        name: this.panelName,
        mode: this.modeName,
        read_only: true,
        entries: entries,
        ratio: this.options.ratio ?? 0.7,
        direction: "horizontal",
        panel_id: this.id,
        show_line_numbers: false,
        show_cursors: true,
        editing_disabled: true,
      });

      if (result.buffer_id !== null) {
        this.state.bufferId = result.buffer_id;
        this.state.splitId = result.split_id ?? null;
        this.state.isOpen = true;
        this.applyHighlighting();

        const count = items.length;
        this.editor.setStatus(`${title}: ${count} item${count !== 1 ? "s" : ""}`);
      } else {
        this.editor.setStatus(`Failed to open ${this.panelName}`);
      }
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      this.editor.setStatus(`Failed to open panel: ${msg}`);
      this.editor.debug(`ResultsPanel error: ${msg}`);
    }
  }

  /**
   * Update the panel content without reopening
   */
  update(showOptions: ShowResultsOptions): void {
    if (!this.state.isOpen || this.state.bufferId === null) {
      return;
    }

    const { title, items, helpText } = showOptions;
    this.state.items = items;
    this.state.title = title;

    const entries = this.buildEntries(title, items, helpText);
    this.state.cachedContent = entries.map(e => e.text).join("");

    this.editor.setVirtualBufferContent(this.state.bufferId, entries);
    this.applyHighlighting();
  }

  /**
   * Close the panel
   */
  close(): void {
    if (!this.state.isOpen) {
      return;
    }

    // Capture values before clearing
    const splitId = this.state.splitId;
    const bufferId = this.state.bufferId;
    const sourceSplitId = this.state.sourceSplitId;

    // Clear state first
    this.state.isOpen = false;
    this.state.bufferId = null;
    this.state.splitId = null;
    this.state.sourceSplitId = null;
    this.state.cachedContent = "";
    this.state.cursorLine = 1;
    this.state.items = [];

    // Close split
    if (splitId !== null) {
      this.editor.closeSplit(splitId);
    }

    // Close buffer
    if (bufferId !== null) {
      this.editor.closeBuffer(bufferId);
    }

    // Focus source
    if (sourceSplitId !== null) {
      this.editor.focusSplit(sourceSplitId);
    }

    // Call user callback
    if (this.options.onClose) {
      this.options.onClose();
    }

    this.editor.setStatus(`${this.panelName} closed`);
  }

  /**
   * Focus the source split (useful for "goto" operations)
   */
  focusSource(): void {
    if (this.state.sourceSplitId !== null) {
      this.editor.focusSplit(this.state.sourceSplitId);
    }
  }

  /**
   * Focus the panel split
   */
  focusPanel(): void {
    if (this.state.splitId !== null) {
      this.editor.focusSplit(this.state.splitId);
    }
  }

  /**
   * Open a file in the source split and jump to location
   *
   * Note: Focus moves to source after this call (similar to diagnostics panel behavior).
   * The user can close the panel via command palette if needed.
   *
   * TODO: Investigate why openFileInSplit doesn't position cursor correctly when file
   * is already open. For now, using focusSplit + openFile which works correctly.
   */
  openInSource(file: string, line: number, column: number): void {
    if (this.state.sourceSplitId === null) return;

    // Focus source split and open file at location
    // This moves focus to source (same behavior as diagnostics panel)
    this.editor.focusSplit(this.state.sourceSplitId);
    this.editor.openFile(file, line, column);
  }

  /**
   * Get the currently selected item
   */
  getSelectedItem(): ResultItem | null {
    const index = this.state.cursorLine - 2; // Line 1 is title, items start at line 2
    if (index >= 0 && index < this.state.items.length) {
      return this.state.items[index];
    }
    return null;
  }

  /**
   * Get the currently selected item index
   */
  getSelectedIndex(): number {
    return Math.max(0, this.state.cursorLine - 2);
  }

  // ============================================
  // Private methods
  // ============================================

  private registerHandlers(): void {
    const self = this;

    // Select handler (Enter)
    (globalThis as Record<string, unknown>)[`${this.handlerPrefix}_select`] = function(): void {
      if (!self.state.isOpen) return;

      const item = self.getSelectedItem();
      const index = self.getSelectedIndex();

      if (item && self.options.onSelect) {
        self.options.onSelect(item, index);
      } else if (!item) {
        self.editor.setStatus("No item selected");
      }
    };

    // Close handler (Escape)
    (globalThis as Record<string, unknown>)[`${this.handlerPrefix}_close`] = function(): void {
      self.close();
    };

    // Cursor movement handler
    (globalThis as Record<string, unknown>)[`${this.handlerPrefix}_cursor_moved`] = function(data: {
      buffer_id: number;
      cursor_id: number;
      old_position: number;
      new_position: number;
      line: number;
    }): void {
      if (!self.state.isOpen || self.state.bufferId === null) return;
      if (data.buffer_id !== self.state.bufferId) return;

      self.state.cursorLine = data.line;
      self.applyHighlighting();

      // Update status
      const index = data.line - 2;
      if (index >= 0 && index < self.state.items.length) {
        self.editor.setStatus(`Item ${index + 1}/${self.state.items.length}`);

        // Call user's cursor move callback
        if (self.options.onCursorMove) {
          self.options.onCursorMove(self.state.items[index], index);
        }
      }
    };

    // Register cursor movement handler
    this.editor.on("cursor_moved", `${this.handlerPrefix}_cursor_moved`);
  }

  private buildEntries(
    title: string,
    items: ResultItem[],
    helpText?: string
  ): TextPropertyEntry[] {
    const entries: TextPropertyEntry[] = [];

    // Title line
    entries.push({
      text: `${title}\n`,
      properties: { type: "title" },
    });

    if (items.length === 0) {
      entries.push({
        text: "  No results\n",
        properties: { type: "empty" },
      });
    } else {
      // Add each item
      for (let i = 0; i < items.length; i++) {
        const item = items[i];
        const label = item.label;
        const desc = item.description ? `  ${item.description}` : "";

        // Truncate to fit
        const maxLen = 100;
        let line = `  ${label}${desc}`;
        if (line.length > maxLen) {
          line = line.slice(0, maxLen - 3) + "...";
        }

        entries.push({
          text: `${line}\n`,
          properties: {
            type: "item",
            index: i,
            location: item.location,
            data: item.data,
          },
        });
      }
    }

    // Help footer
    entries.push({
      text: "\n",
      properties: { type: "blank" },
    });
    entries.push({
      text: helpText ?? "Enter:select | Esc:close\n",
      properties: { type: "help" },
    });

    return entries;
  }

  private applyHighlighting(): void {
    if (this.state.bufferId === null) return;

    const bufferId = this.state.bufferId;
    this.editor.clearNamespace(bufferId, this.namespace);

    if (!this.state.cachedContent) return;

    const lines = this.state.cachedContent.split("\n");
    let byteOffset = 0;

    for (let lineIdx = 0; lineIdx < lines.length; lineIdx++) {
      const line = lines[lineIdx];
      const lineStart = byteOffset;
      const lineEnd = byteOffset + line.length;
      const lineNumber = lineIdx + 1;
      const isCurrentLine = lineNumber === this.state.cursorLine;
      const isItemLine = lineNumber >= 2 && lineNumber < 2 + this.state.items.length;

      // Highlight current line if it's an item line
      if (isCurrentLine && isItemLine && line.trim() !== "") {
        this.editor.addOverlay(
          bufferId, this.namespace, lineStart, lineEnd,
          colors.selected[0], colors.selected[1], colors.selected[2],
          true, true, false
        );
      }

      // Title line highlighting
      if (lineNumber === 1) {
        this.editor.addOverlay(
          bufferId, this.namespace, lineStart, lineEnd,
          colors.title[0], colors.title[1], colors.title[2],
          true, true, false
        );
      }

      // Help line highlighting (dimmed)
      if (line.startsWith("Enter:") || line.includes("|")) {
        this.editor.addOverlay(
          bufferId, this.namespace, lineStart, lineEnd,
          colors.help[0], colors.help[1], colors.help[2],
          false, true, false
        );
      }

      // Highlight location patterns (file:line:col)
      const locMatch = line.match(/^\s+(\S+:\d+:\d+)/);
      if (locMatch && !isCurrentLine) {
        const locStart = lineStart + line.indexOf(locMatch[1]);
        const locEnd = locStart + locMatch[1].length;
        this.editor.addOverlay(
          bufferId, this.namespace, locStart, locEnd,
          colors.location[0], colors.location[1], colors.location[2],
          false, false, false
        );
      }

      byteOffset += line.length + 1;
    }
  }
}

/**
 * Get the relative path for display
 */
export function getRelativePath(editor: EditorAPI, filePath: string): string {
  const cwd = editor.getCwd();
  if (filePath.startsWith(cwd)) {
    return filePath.slice(cwd.length + 1);
  }
  return filePath;
}

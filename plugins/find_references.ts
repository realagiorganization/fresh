/// <reference path="./lib/fresh.d.ts" />

import { ResultsPanel, ResultItem, getRelativePath } from "./lib/results-panel.ts";

const editor = getEditor();

/**
 * Find References Plugin
 *
 * Displays LSP find references results using the ResultsPanel abstraction.
 * The plugin provides data (reference locations) and the ResultsPanel
 * handles all UI concerns (navigation, selection highlighting, keybindings).
 */

// Maximum number of results to display
const MAX_RESULTS = 100;

// Reference item structure from LSP
interface ReferenceItem {
  file: string;
  line: number;
  column: number;
}

// Line text cache for previews
const lineCache: Map<string, string[]> = new Map();

// Create the results panel
const panel = new ResultsPanel(editor, "references", {
  ratio: 0.7,
  onSelect: (item) => {
    if (item.location) {
      // Open file in source split, keeping focus on panel
      panel.openInSource(item.location.file, item.location.line, item.location.column);
      const displayPath = getRelativePath(editor, item.location.file);
      editor.setStatus(`Jumped to ${displayPath}:${item.location.line}`);
    }
  },
  onClose: () => {
    lineCache.clear();
  },
});

/**
 * Load line text for references (for preview display)
 */
async function loadLineTexts(references: ReferenceItem[]): Promise<Map<string, string>> {
  const lineTexts = new Map<string, string>();

  // Group references by file
  const fileRefs: Map<string, ReferenceItem[]> = new Map();
  for (const ref of references) {
    if (!fileRefs.has(ref.file)) {
      fileRefs.set(ref.file, []);
    }
    fileRefs.get(ref.file)!.push(ref);
  }

  // Load each file and extract lines
  for (const [filePath, refs] of fileRefs) {
    try {
      let lines = lineCache.get(filePath);
      if (!lines) {
        const content = await editor.readFile(filePath);
        lines = content.split("\n");
        lineCache.set(filePath, lines);
      }

      for (const ref of refs) {
        const lineIndex = ref.line - 1;
        if (lineIndex >= 0 && lineIndex < lines.length) {
          const key = `${ref.file}:${ref.line}:${ref.column}`;
          lineTexts.set(key, lines[lineIndex]);
        }
      }
    } catch {
      // If file can't be read, skip
    }
  }

  return lineTexts;
}

/**
 * Convert LSP references to ResultItems for display
 */
function referencesToItems(
  references: ReferenceItem[],
  lineTexts: Map<string, string>
): ResultItem[] {
  return references.map(ref => {
    const displayPath = getRelativePath(editor, ref.file);
    const key = `${ref.file}:${ref.line}:${ref.column}`;
    const lineText = lineTexts.get(key) || "";
    const trimmedLine = lineText.trim();

    // Format: "path:line:col  preview"
    const location = `${displayPath}:${ref.line}:${ref.column}`;
    const maxLocationLen = 50;
    const truncatedLocation = location.length > maxLocationLen
      ? "..." + location.slice(-(maxLocationLen - 3))
      : location;

    const maxPreviewLen = 50;
    const preview = trimmedLine.length > maxPreviewLen
      ? trimmedLine.slice(0, maxPreviewLen - 3) + "..."
      : trimmedLine;

    return {
      label: truncatedLocation,
      description: preview,
      location: {
        file: ref.file,
        line: ref.line,
        column: ref.column,
      },
    };
  });
}

/**
 * Show references panel with the given results
 */
async function showReferences(symbol: string, references: ReferenceItem[]): Promise<void> {
  // Limit results
  const limitedRefs = references.slice(0, MAX_RESULTS);

  // Clear and reload line cache
  lineCache.clear();
  const lineTexts = await loadLineTexts(limitedRefs);

  // Convert to ResultItems
  const items = referencesToItems(limitedRefs, lineTexts);

  // Build title
  const count = references.length;
  const limitNote = count > MAX_RESULTS ? ` (showing first ${MAX_RESULTS})` : "";
  const title = `References to '${symbol}': ${count}${limitNote}`;

  // Show panel
  await panel.show({
    title,
    items,
    helpText: "Enter:goto | Esc:close",
  });
}

// Handle lsp_references hook
globalThis.on_lsp_references = function(data: { symbol: string; locations: ReferenceItem[] }): void {
  editor.debug(`Received ${data.locations.length} references for '${data.symbol}'`);

  if (data.locations.length === 0) {
    editor.setStatus(`No references found for '${data.symbol}'`);
    return;
  }

  showReferences(data.symbol, data.locations);
};

// Register the hook handler
editor.on("lsp_references", "on_lsp_references");

// Export close function for command palette
globalThis.hide_references_panel = function(): void {
  panel.close();
};

// Register commands
editor.registerCommand(
  "%cmd.show_references",
  "%cmd.show_references_desc",
  "show_references_panel",
  "normal"
);

editor.registerCommand(
  "%cmd.hide_references",
  "%cmd.hide_references_desc",
  "hide_references_panel",
  "normal"
);

// Plugin initialization
editor.setStatus("Find References plugin ready");
editor.debug("Find References plugin initialized");

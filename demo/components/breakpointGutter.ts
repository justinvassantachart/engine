'use client';

import { Facet, StateField } from '@codemirror/state';
import { Decoration, DecorationSet, EditorView, gutter, GutterMarker } from '@codemirror/view';

/** Configuration passed from React: breakpoint lines, paused line, and toggle callback. */
export interface BreakpointGutterConfig {
  breakpointLines: ReadonlySet<number>;
  pausedLine: number | null;
  onToggleLine: (line: number) => void;
}

export const breakpointGutterConfigFacet = Facet.define<
  BreakpointGutterConfig,
  BreakpointGutterConfig
>({
  combine: (values) => (values.length > 0 ? values[values.length - 1] : getDefaultConfig()),
});

function getDefaultConfig(): BreakpointGutterConfig {
  return {
    breakpointLines: new Set(),
    pausedLine: null,
    onToggleLine: () => {},
  };
}

const breakpointMarker = new (class extends GutterMarker {
  toDOM() {
    const el = document.createElement('div');
    el.className = 'cm-breakpoint-marker';
    el.setAttribute('aria-label', 'Breakpoint');
    el.title = 'Breakpoint (click to remove)';
    return el;
  }
})();

/** Extension: gutter that shows breakpoint markers and highlights the paused line. */
export function breakpointGutterExtension(config: BreakpointGutterConfig) {
  return [
    breakpointGutterConfigFacet.of(config),
    gutter({
      class: 'cm-breakpoint-gutter',
      lineMarker: (view, line) => {
        const c = view.state.facet(breakpointGutterConfigFacet);
        const lineNumber = view.state.doc.lineAt(line.from).number;
        if (c.breakpointLines.has(lineNumber)) return breakpointMarker;
        return null;
      },
      initialSpacer: () => breakpointMarker,
      domEventHandlers: {
        mousedown: (view, line) => {
          const lineNumber = view.state.doc.lineAt(line.from).number;
          const c = view.state.facet(breakpointGutterConfigFacet);
          c.onToggleLine(lineNumber);
          return true;
        },
      },
    }),
    EditorView.baseTheme({
      '.cm-breakpoint-gutter': {
        width: '20px',
        minWidth: '20px',
      },
      '.cm-breakpoint-gutter .cm-gutterElement': {
        paddingLeft: '4px',
        cursor: 'pointer',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
      },
      '.cm-breakpoint-marker': {
        width: '12px',
        height: '12px',
        borderRadius: '50%',
        backgroundColor: '#ef4444',
        border: '2px solid rgba(255,255,255,0.3)',
        boxSizing: 'border-box',
      },
      '.cm-breakpoint-gutter .cm-gutterElement:hover .cm-breakpoint-marker': {
        backgroundColor: '#f87171',
      },
      '.cm-paused-line': {
        backgroundColor: 'rgba(234, 179, 8, 0.25)',
      },
      '.cm-paused-line-gutter': {
        backgroundColor: 'rgba(234, 179, 8, 0.35)',
      },
    }),
    pausedLineHighlightExtension(),
  ];
}

/** Highlights the current paused line in the editor (reads from breakpoint config facet). */
function pausedLineHighlightExtension() {
  const field = StateField.define<DecorationSet>({
    create(state) {
      const c = state.facet(breakpointGutterConfigFacet);
      return c.pausedLine != null ? lineDecorationSet(state.doc, c.pausedLine) : Decoration.none;
    },
    update(_set, tr) {
      const c = tr.state.facet(breakpointGutterConfigFacet);
      return c.pausedLine != null ? lineDecorationSet(tr.state.doc, c.pausedLine) : Decoration.none;
    },
    provide: (f) => EditorView.decorations.from(f),
  });
  return [field];
}

function lineDecorationSet(
  doc: { line: (n: number) => { from: number; to: number } },
  line: number
) {
  try {
    const target = doc.line(line);
    return Decoration.set([Decoration.line({ class: 'cm-paused-line' }).range(target.from)]);
  } catch {
    return Decoration.none;
  }
}

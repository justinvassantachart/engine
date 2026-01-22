'use client';

import { Box } from '@mui/material';
import '@xterm/xterm/css/xterm.css';
import React from 'react';

type TerminalHandle = {
  write: (data: string | Uint8Array) => void;
  writeln: (data: string | Uint8Array) => void;
  clear: () => void;
  focus: () => void;
};

type TerminalProps = {
  /** Visual height of the terminal container. */
  height?: number | string;
};

// Xterm configuration (cursor behavior, font, initial rows).
// We could play more with this to make the terminal look nicer.
const terminalOptions = {
  fontFamily: 'monospace',
  fontSize: 14,
  lineHeight: 1.25,
  cursorBlink: true,
  cursorStyle: 'block' as const,
  rows: 1,
};

const Terminal = React.forwardRef<TerminalHandle, TerminalProps>(({ height = 180 }, ref) => {
  // DOM mount point for Xterm.
  const terminalEl = React.useRef<HTMLDivElement | null>(null);
  // Xterm instance. We keep this in a ref to avoid re-renders on output.
  const terminalRef = React.useRef<import('@xterm/xterm').Terminal | null>(null);

  React.useImperativeHandle(
    ref,
    () => ({
      // Use these methods from parent components to write output.
      write: (data) => terminalRef.current?.write(data),
      writeln: (data) => terminalRef.current?.writeln(data),
      clear: () => terminalRef.current?.clear(),
      focus: () => terminalRef.current?.focus(),
    }),
    []
  );

  React.useEffect(() => {
    let disposed = false;
    let fitAddon: import('@xterm/addon-fit').FitAddon | null = null;

    // Refit terminal when the window resizes.
    const onResize = () => fitAddon?.fit();

    const setupTerminal = async () => {
      if (!terminalEl.current) return;

      // NOTE: Dynamic import avoids "self is not defined" during SSR.
      // If we ever move to non-SSR or split to client-only routes,
      // we can switch to static imports.
      // this could also be solved like this:
      //   const Terminal = dynamic(() => import("@/components/Terminal"), {
      //     ssr: false,
      //   });
      const [{ Terminal: XTerm }, { FitAddon }] = await Promise.all([
        import('@xterm/xterm'),
        import('@xterm/addon-fit'),
      ]);

      if (disposed || !terminalEl.current) return;

      // Initialize Xterm instance.
      // This is the spot to add addons like Unicode/Weird width handling later.
      const term = new XTerm(terminalOptions);
      fitAddon = new FitAddon();
      term.loadAddon(fitAddon);
      term.open(terminalEl.current);
      fitAddon.fit();

      terminalRef.current = term;
      window.addEventListener('resize', onResize);
    };

    void setupTerminal();

    return () => {
      disposed = true;
      window.removeEventListener('resize', onResize);
      // Ensure Xterm frees all event listeners and DOM nodes.
      terminalRef.current?.dispose();
      terminalRef.current = null;
    };
  }, []);

  return (
    <Box
      ref={terminalEl}
      sx={{
        height,
        width: '100%',
        px: 2,
        pb: 1.5,
        pt: 1,
        // Container styling — tweak background/border to match the editor theme.
        background: 'rgba(0, 0, 0, 0.35)',
        borderTop: '1px solid rgba(255, 255, 255, 0.08)',
        '& .xterm-viewport': {
          overflowY: 'auto',
        },
        // TODO: Switch to "auto" when we support stdin and selection.
        '& .xterm-screen': {
          pointerEvents: 'none',
        },
      }}
    />
  );
});

Terminal.displayName = 'Terminal';

export type { TerminalHandle };
export default Terminal;

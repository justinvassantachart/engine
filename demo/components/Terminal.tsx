'use client';

import { Box } from '@mui/material';
import '@xterm/xterm/css/xterm.css';
import React from 'react';

type TerminalHandle = {
  write: (data: string | Uint8Array) => void;
  writeln: (data: string | Uint8Array) => void;
  clear: () => void;
  focus: () => void;
  /** Get the underlying xterm.js Terminal instance for attaching event handlers */
  getTerminal: () => import('@xterm/xterm').Terminal | null;
  /** Enable input by allowing pointer events on the terminal screen */
  enableInput: () => void;
  /** Disable input by blocking pointer events on the terminal screen */
  disableInput: () => void;
  /** Resize the terminal to fit its container (call when container size changes) */
  resize: () => void;
};

type TerminalProps = {
  /** Visual height of the terminal container. */
  height?: number | string;
};

// Xterm configuration (cursor behavior, font, initial rows).
// We could play more with this to make the terminal look nicer.
const terminalOptions = {
  fontFamily:
    '"JetBrains Mono", ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace',
  fontSize: 13,
  lineHeight: 1.4,
  cursorBlink: true,
  cursorStyle: 'bar' as const,
  rows: 6,
  theme: {
    background: '#0b0e14',
    foreground: '#e5e7eb',
    cursor: '#a5b4fc',
    black: '#0b0e14',
    brightBlack: '#334155',
    red: '#f87171',
    brightRed: '#fca5a5',
    green: '#34d399',
    brightGreen: '#6ee7b7',
    yellow: '#facc15',
    brightYellow: '#fde047',
    blue: '#60a5fa',
    brightBlue: '#93c5fd',
    magenta: '#c084fc',
    brightMagenta: '#e9d5ff',
    cyan: '#22d3ee',
    brightCyan: '#67e8f9',
    white: '#e5e7eb',
    brightWhite: '#f8fafc',
  },
};

const Terminal = React.forwardRef<TerminalHandle, TerminalProps>(({ height = 180 }, ref) => {
  // DOM mount point for Xterm.
  const terminalEl = React.useRef<HTMLDivElement | null>(null);
  // Xterm instance. We keep this in a ref to avoid re-renders on output.
  const terminalRef = React.useRef<import('@xterm/xterm').Terminal | null>(null);
  // Reference to the container element for toggling pointer events
  const containerRef = React.useRef<HTMLDivElement | null>(null);
  // Reference to FitAddon for resizing
  const fitAddonRef = React.useRef<import('@xterm/addon-fit').FitAddon | null>(null);

  // Use useImperativeHandle to expose the terminal methods to the parent component (CodeEditor)
  React.useImperativeHandle(
    ref,
    () => ({
      // Use these methods from parent components to write output.
      write: (data) => terminalRef.current?.write(data),
      writeln: (data) => terminalRef.current?.writeln(data),
      clear: () => terminalRef.current?.clear(),
      focus: () => terminalRef.current?.focus(),
      getTerminal: () => terminalRef.current,
      enableInput: () => {
        if (containerRef.current) {
          const screen = containerRef.current.querySelector('.xterm-screen');
          if (screen instanceof HTMLElement) {
            screen.style.pointerEvents = 'auto';
          }
        }
      },
      disableInput: () => {
        if (containerRef.current) {
          const screen = containerRef.current.querySelector('.xterm-screen');
          if (screen instanceof HTMLElement) {
            screen.style.pointerEvents = 'none';
          }
        }
      },
      resize: () => {
        // Resize terminal to fit its container
        fitAddonRef.current?.fit();
      },
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
      fitAddonRef.current = fitAddon;
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
      fitAddonRef.current = null;
    };
  }, []);

  // Resize terminal when height prop changes
  React.useEffect(() => {
    if (fitAddonRef.current) {
      // Use requestAnimationFrame to ensure DOM has updated
      requestAnimationFrame(() => {
        fitAddonRef.current?.fit();
      });
    }
  }, [height]);

  return (
    <Box
      // ref to the terminal element
      ref={(el: HTMLDivElement | null) => {
        terminalEl.current = el;
        containerRef.current = el;
      }}
      sx={{
        height,
        width: '100%',
        px: 2,
        pb: 1.5,
        pt: 1,
        // Container styling — tweak background/border to match the editor theme.
        background: 'linear-gradient(180deg, rgba(10, 12, 18, 0.75) 0%, rgba(5, 7, 12, 0.9) 100%)',
        '& .xterm-viewport': {
          overflowY: 'auto',
        },
        // Input is disabled by default. Use enableInput()/disableInput() to toggle.
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

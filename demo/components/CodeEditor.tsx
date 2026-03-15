'use client';

import { cpp } from '@codemirror/lang-cpp';
import { oneDark } from '@codemirror/theme-one-dark';
import { lineNumbers } from '@codemirror/view';
import PlayArrowIcon from '@mui/icons-material/PlayArrow';
import StopIcon from '@mui/icons-material/Stop';
import { Box, Button, FormControl, InputLabel, MenuItem, Select, Typography } from '@mui/material';
import CodeMirror from '@uiw/react-codemirror';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { LocationInfo, Runtime } from 'runtime';
import type { Breakpoint, BreakpointHit } from 'runtime';

import { breakpointGutterExtension } from '@/components/breakpointGutter';
import Terminal, { TerminalHandle } from '@/components/Terminal';
import VariablesPanel from '@/components/VariablesPanel';
import { DEFAULT_SOURCE_CODE, DEMO_SOURCE_FILE } from '@/config/demoConfig';

type Language = 'C' | 'C++';

export default function CodeEditor() {
  const [code, setCode] = useState<string>(DEFAULT_SOURCE_CODE);
  const [isRunning, setIsRunning] = useState(false);
  const [isPaused, setIsPaused] = useState(false);
  const [pausedLocation, setPausedLocation] = useState<LocationInfo | null>(null);
  /** Full breakpoint hit for Variables / Call stack panel (frames + variables). */
  const [pausedHit, setPausedHit] = useState<BreakpointHit | null>(null);
  const [breakpointLines, setBreakpointLines] = useState<number[]>([]);
  const [language, setLanguage] = useState<Language>('C++');
  const [terminalHeight, setTerminalHeight] = useState<number>(170);
  const terminalRef = useRef<TerminalHandle | null>(null);
  // Ref to track running state for stdin handler (avoids stale closure)
  const isRunningRef = useRef<boolean>(false);
  // Ref to store the current runtime instance for stopping execution (Ctrl+C)
  const runtimeRef = useRef<Runtime | null>(null);
  // Ref to track if execution was stopped by user (Ctrl+C)
  const wasStoppedByUserRef = useRef<boolean>(false);
  // Refs for drag resizing
  const isDraggingRef = useRef<boolean>(false);
  const containerRef = useRef<HTMLDivElement | null>(null);
  // Ref to store onData handler for cleanup (prevents duplicate handlers)
  const onDataHandlerRef = useRef<{ dispose: () => void } | null>(null);
  // Map of line -> runtime Breakpoint for the current run (so we can add/remove while paused).
  const runtimeBreakpointsRef = useRef<Map<number, Breakpoint>>(new Map());

  const handleLanguageChange = (newLanguage: Language) => {
    setLanguage(newLanguage);
  };

  /**
   * Handle Stop Button Click
   *
   * Stops the currently running program execution.
   */
  const handleStop = () => {
    if (runtimeRef.current) {
      runtimeRef.current.stop();
    }
  };

  const handleContinue = () => {
    runtimeRef.current?.debugger.resume();
    setIsPaused(false);
    setPausedLocation(null);
    setPausedHit(null);
  };

  const toggleBreakpoint = useCallback((line: number) => {
    setBreakpointLines((prev) => {
      const hasLine = prev.includes(line);
      const next = hasLine ? prev.filter((l) => l !== line) : [...prev, line].sort((a, b) => a - b);

      // If a runtime is active, mirror the change into the debugger immediately so
      // adding/removing breakpoints while paused takes effect without restarting.
      const rt = runtimeRef.current;
      if (rt) {
        const map = runtimeBreakpointsRef.current;
        if (!hasLine && !map.has(line)) {
          const bp = rt.debugger.addBreakpoint(String(line));
          map.set(line, bp);
        } else if (hasLine) {
          const existing = map.get(line);
          if (existing) {
            rt.debugger.removeBreakpoint(existing);
            map.delete(line);
          }
        }
      }

      return next;
    });
  }, []);

  /**
   * Handle Run Button Click
   *
   * Compiles and runs the user's code, setting up stdin/stdout/stderr streams.
   * Stdin input is captured from the terminal and only accepted when the program
   * is actively running and waiting for input.
   */
  const handleRun = async () => {
    setIsRunning(true);
    isRunningRef.current = true;
    wasStoppedByUserRef.current = false;

    // Create AbortController to cancel pipeTo connections when stopping
    // Declared outside try block so it's accessible in finally
    let abortController: AbortController | null = null;

    try {
      // Clear terminal output for a fresh run
      terminalRef.current?.clear();
      terminalRef.current?.writeln('Running...');

      const rt = Runtime.create('c');
      rt.debug = breakpointLines.length > 0; // fast path when no breakpoints
      runtimeRef.current = rt;
      runtimeBreakpointsRef.current = new Map();

      // Set up stdout/stderr streams to write to the terminal
      // Convert lone \n to \r\n for proper terminal display (xterm.js expects \r\n)

      // Create AbortController to cancel pipeTo connections when stopping
      abortController = new AbortController();
      const signal = abortController.signal;

      const terminal = new WritableStream<Uint8Array>({
        write: (chunk) => {
          // Decode bytes to string, normalize newlines, then write
          const decoder = new TextDecoder();
          const text = decoder.decode(chunk);
          // Replace any \n (not already preceded by \r) with \r\n
          const normalized = text.replace(/\r?\n/g, '\r\n');
          terminalRef.current?.write(normalized);
        },
        abort: () => {
          // Stream aborted - cleanup
        },
      });

      const stderrStream = new WritableStream<Uint8Array>({
        write: (chunk) => {
          // Decode bytes to string, normalize newlines, then write
          const decoder = new TextDecoder();
          const text = decoder.decode(chunk);
          const normalized = text.replace(/\r?\n/g, '\r\n');
          terminalRef.current?.write(normalized);
          // No timeout logic needed - worker will always send 'stop' message
          // (both on success and on errors/panics)
        },
        abort: () => {
          // Stream aborted - cleanup
        },
      });

      // Store pipeTo promises so we can abort them if needed
      // Pipes are set up and will be aborted via abortController when needed
      rt.stdout.pipeTo(terminal, { signal }).catch(() => {
        // Ignore abort errors
      });
      rt.stderr.pipeTo(stderrStream, { signal }).catch(() => {
        // Ignore abort errors
      });
      rt.fs = { [DEMO_SOURCE_FILE]: code };

      // Seed debugger breakpoints for this run from the current React state.
      for (const line of breakpointLines) {
        const bp = rt.debugger.addBreakpoint(String(line));
        runtimeBreakpointsRef.current.set(line, bp);
      }

      const dbg = rt.debugger;
      dbg.on('breakpoint', (hit) => {
        setPausedHit(hit);
        setIsPaused(true);
        setPausedLocation(hit.location);
        terminalRef.current?.writeln(`\r\nPaused at ${hit.location.file}:${hit.location.line}`);
      });

      // Get the underlying xterm.js terminal instance for stdin handling
      const term = terminalRef.current?.getTerminal();
      if (!term) {
        throw new Error('Terminal not initialized');
      }

      // Enable input on the terminal (allows pointer events)
      terminalRef.current?.enableInput();

      // Set up stdin input handling
      // Buffer to store user input before sending to stdin
      let stdinBuffer = '';
      const encoder = new TextEncoder();
      const stdinWriter = rt.stdin.getWriter();

      // Dispose any existing handler first to prevent duplicates
      if (onDataHandlerRef.current) {
        onDataHandlerRef.current.dispose();
        onDataHandlerRef.current = null;
      }

      /**
       * Handle keyboard input from the terminal.
       * Only processes input when code is running (checked via isRunningRef).
       * Supports control sequences:
       * - Ctrl+C (\x03): Stop execution
       * - Ctrl+D (\x04): Send EOF to stdin
       * - Ctrl+L (\x0c): Clear terminal screen
       */
      const onData = term.onData((data: string) => {
        // Only accept input when code is actively running
        if (!isRunningRef.current) return;

        // Control sequences (single character codes)
        if (data === '\x03') {
          // Ctrl+C: Stop execution
          term.write('^C\r\n');
          wasStoppedByUserRef.current = true;
          // Stop the runtime execution
          // Note: TypeScript types need to be regenerated (npm run build) to recognize stop() method
          const rt = runtimeRef.current as Runtime & { stop?: () => void };
          if (rt?.stop) {
            rt.stop();
          }
          // Immediately update UI state when stopped (don't wait for run() to complete)
          isRunningRef.current = false;
          setIsRunning(false);
          terminalRef.current?.disableInput();
          terminalRef.current?.writeln('Execution stopped by user (Ctrl+C)');
          return;
        } else if (data === '\x04') {
          // Ctrl+D: Send EOF (End of File) to stdin
          // EOF is typically represented as 0x04 or by closing the stream
          term.write('^D\r\n');
          stdinWriter.write(encoder.encode('\x04')); // Send EOF character
          stdinBuffer = '';
          return;
        } else if (data === '\x0c') {
          // Ctrl+L: Clear terminal screen (form feed)
          terminalRef.current?.clear();
          stdinBuffer = '';
          return;
        }

        // Regular input handling
        if (data === '\r') {
          // Enter key pressed: send buffered input to stdin with newline
          term.write('\r\n');
          stdinWriter.write(encoder.encode(`${stdinBuffer}\n`));
          stdinBuffer = '';
        } else if (data === '\u007f') {
          // Backspace: remove last character from buffer and terminal
          if (stdinBuffer.length > 0) {
            stdinBuffer = stdinBuffer.slice(0, -1);
            term.write('\b \b');
          }
        } else if (
          // Ignore arrow keys and other control sequences
          data === '\x1b[A' || // Up arrow
          data === '\x1b[B' || // Down arrow
          data === '\x1b[C' || // Right arrow
          data === '\x1b[D' // Left arrow
        ) {
          return; // Ignore arrow keys
        } else {
          // Regular character: add to buffer and echo to terminal
          stdinBuffer += data;
          term.write(data);
        }
      });

      // Store handler reference for cleanup
      onDataHandlerRef.current = onData;

      // Run the program
      // Worker will always send 'stop' message (on success or error)
      await rt.run();
    } catch (error) {
      if (wasStoppedByUserRef.current) {
        return;
      }
      const message = error instanceof Error ? error.message : String(error);
      terminalRef.current?.writeln(`\r\nError: ${message}`);
    } finally {
      // Clean up: dispose of the stdin event handler (must be in finally to ensure cleanup)
      // This prevents duplicate handlers on subsequent runs
      if (onDataHandlerRef.current) {
        onDataHandlerRef.current.dispose();
        onDataHandlerRef.current = null;
      }
      // Abort any active pipeTo connections to prevent duplicate output
      if (abortController) {
        abortController.abort();
      }
      runtimeRef.current = null;
      isRunningRef.current = false;
      terminalRef.current?.disableInput();
      setIsRunning(false);
      setIsPaused(false);
      setPausedLocation(null);
      setPausedHit(null);
    }
  };

  // Handle drag resizing of terminal
  useEffect(() => {
    const handleMouseMove = (e: MouseEvent) => {
      if (!isDraggingRef.current || !containerRef.current) return;

      const containerRect = containerRef.current.getBoundingClientRect();
      const containerHeight = containerRect.height;
      const mouseY = e.clientY;
      const relativeY = mouseY - containerRect.top;

      // Calculate new terminal height (from bottom)
      // Min height: 100px, Max height: containerHeight - 200px (leave room for editor)
      const newHeight = Math.max(100, Math.min(containerHeight - 200, containerHeight - relativeY));
      setTerminalHeight(newHeight);

      // Resize terminal immediately
      requestAnimationFrame(() => {
        terminalRef.current?.resize();
      });
    };

    const handleMouseUp = () => {
      if (isDraggingRef.current) {
        isDraggingRef.current = false;
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
        // Final resize after drag ends
        requestAnimationFrame(() => {
          terminalRef.current?.resize();
        });
      }
    };

    window.addEventListener('mousemove', handleMouseMove);
    window.addEventListener('mouseup', handleMouseUp);

    return () => {
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
    };
  }, []);

  const handleMouseDown = (e: React.MouseEvent) => {
    e.preventDefault();
    isDraggingRef.current = true;
    document.body.style.cursor = 'row-resize';
    document.body.style.userSelect = 'none';
  };

  const breakpointGutterConfig = useMemo(
    () => ({
      breakpointLines: new Set(breakpointLines),
      pausedLine: pausedLocation?.line ?? null,
      onToggleLine: toggleBreakpoint,
    }),
    [breakpointLines, pausedLocation?.line, toggleBreakpoint]
  );

  const extensions = useMemo(
    () => [breakpointGutterExtension(breakpointGutterConfig), lineNumbers(), cpp()],
    [breakpointGutterConfig]
  );

  return (
    <Box sx={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <Box
        sx={{
          px: 3,
          py: 1.75,
          borderBottom: '1px solid',
          borderColor: 'rgba(148, 163, 184, 0.15)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 3,
          background:
            'linear-gradient(180deg, rgba(18, 18, 24, 0.9) 0%, rgba(12, 12, 16, 0.6) 100%)',
          backdropFilter: 'blur(8px)',
        }}
      >
        <Box sx={{ display: 'flex', alignItems: 'center', gap: 2, flexWrap: 'wrap' }}>
          <Box sx={{ display: 'flex', flexDirection: 'column', gap: 0.25 }}>
            <Box sx={{ display: 'flex', alignItems: 'center', gap: 1 }}>
              <Typography variant="subtitle2" sx={{ fontWeight: 600 }}>
                Runtime Playground
              </Typography>
              <Box
                sx={{
                  px: 1,
                  py: 0.3,
                  borderRadius: 1,
                  fontSize: '0.65rem',
                  letterSpacing: '0.12em',
                  textTransform: 'uppercase',
                  color: '#c7d2fe',
                  background: 'rgba(99, 102, 241, 0.2)',
                  border: '1px solid rgba(99, 102, 241, 0.45)',
                }}
              >
                Demo
              </Box>
            </Box>
            <Typography variant="caption" sx={{ color: 'rgba(255, 255, 255, 0.55)' }}>
              Edit, run, and review output instantly
            </Typography>
          </Box>
          <FormControl size="small" sx={{ minWidth: 150 }}>
            <InputLabel sx={{ fontSize: '0.875rem' }}>Language</InputLabel>
            <Select
              value={language}
              label="Language"
              onChange={(e) => handleLanguageChange(e.target.value as Language)}
              sx={{
                fontSize: '0.875rem',
                '& .MuiOutlinedInput-notchedOutline': {
                  borderColor: 'rgba(148, 163, 184, 0.35)',
                },
                '&:hover .MuiOutlinedInput-notchedOutline': {
                  borderColor: 'rgba(148, 163, 184, 0.55)',
                },
              }}
            >
              <MenuItem value="C">C</MenuItem>
              <MenuItem value="C++">C++</MenuItem>
            </Select>
          </FormControl>
        </Box>
        <Box sx={{ display: 'flex', alignItems: 'center', gap: 2 }}>
          <Typography
            variant="caption"
            sx={{
              color: 'rgba(255, 255, 255, 0.55)',
              fontSize: '0.75rem',
              fontFamily:
                'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace',
              fontVariantNumeric: 'tabular-nums',
            }}
          >
            {code.split('\n').length} lines
          </Typography>

          {isPaused && (
            <Button
              variant="contained"
              size="small"
              startIcon={<PlayArrowIcon />}
              onClick={handleContinue}
              sx={{
                minWidth: 100,
                textTransform: 'none',
                background: 'linear-gradient(135deg, #22c55e 0%, #16a34a 100%)',
                border: '1px solid rgba(255, 255, 255, 0.12)',
                boxShadow: '0 10px 25px rgba(34, 197, 94, 0.3)',
                '&:hover': {
                  background: 'linear-gradient(135deg, #16a34a 0%, #15803d 100%)',
                },
              }}
            >
              Continue
            </Button>
          )}

          <Button
            variant="contained"
            size="small"
            startIcon={isRunning ? <StopIcon /> : <PlayArrowIcon />}
            onClick={isRunning ? handleStop : handleRun}
            disabled={isPaused}
            sx={{
              minWidth: 100,
              textTransform: 'none',
              ...(isRunning
                ? {
                    background: 'linear-gradient(135deg, #ef4444 0%, #dc2626 100%)',
                    border: '1px solid rgba(255, 255, 255, 0.12)',
                    boxShadow: '0 10px 25px rgba(239, 68, 68, 0.3)',
                    '&:hover': {
                      background: 'linear-gradient(135deg, #dc2626 0%, #b91c1c 100%)',
                    },
                  }
                : {
                    background: 'linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%)',
                    border: '1px solid rgba(255, 255, 255, 0.12)',
                    boxShadow: '0 10px 25px rgba(99, 102, 241, 0.3)',
                    '&:hover': {
                      background: 'linear-gradient(135deg, #5855eb 0%, #7c3aed 100%)',
                    },
                  }),
            }}
          >
            {isRunning ? 'Stop' : 'Run'}
          </Button>
        </Box>
      </Box>
      <Box
        ref={containerRef}
        sx={{
          flex: 1,
          overflow: 'hidden',
          display: 'flex',
          flexDirection: 'row',
          position: 'relative',
        }}
      >
        {/* Left: Editor + Output */}
        <Box sx={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          {/* Code Editor - takes remaining space */}
          <Box
            sx={{ flex: 1, overflow: 'auto', background: 'rgba(10, 12, 18, 0.6)', minHeight: 0 }}
          >
            <CodeMirror
              value={code}
              height="100%"
              theme={oneDark}
              extensions={extensions}
              onChange={(value) => setCode(value)}
              basicSetup={{
                lineNumbers: false,
                foldGutter: true,
                dropCursor: false,
                allowMultipleSelections: false,
                indentOnInput: true,
                bracketMatching: true,
                closeBrackets: true,
                autocompletion: true,
                highlightSelectionMatches: true,
              }}
            />
          </Box>

          {/* Draggable Resizer */}
          <Box
            onMouseDown={handleMouseDown}
            sx={{
              height: '4px',
              cursor: 'row-resize',
              backgroundColor: 'rgba(148, 163, 184, 0.15)',
              position: 'relative',
              '&:hover': {
                backgroundColor: 'rgba(148, 163, 184, 0.3)',
              },
              '&::before': {
                content: '""',
                position: 'absolute',
                top: '-2px',
                left: 0,
                right: 0,
                height: '8px',
                cursor: 'row-resize',
              },
            }}
          />

          {/* Output Header */}
          <Box
            sx={{
              px: 2.5,
              py: 0.75,
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'space-between',
              borderTop: '1px solid rgba(148, 163, 184, 0.15)',
              background: 'rgba(9, 11, 16, 0.75)',
              flexShrink: 0,
            }}
          >
            <Typography
              variant="caption"
              sx={{
                color: 'rgba(255, 255, 255, 0.55)',
                letterSpacing: '0.12em',
                textTransform: 'uppercase',
              }}
            >
              Output
            </Typography>
            <Typography
              variant="caption"
              sx={{
                color: isPaused ? '#22c55e' : isRunning ? '#fbbf24' : 'rgba(255, 255, 255, 0.4)',
              }}
            >
              {isPaused
                ? `Paused at ${pausedLocation?.file}:${pausedLocation?.line}`
                : isRunning
                  ? 'Running'
                  : 'Ready'}
            </Typography>
          </Box>

          {/* Terminal - fixed height, resizable */}
          <Terminal ref={terminalRef} height={terminalHeight} />
        </Box>

        {/* Right: Variables / Call stack */}
        <Box
          sx={{
            width: 280,
            flexShrink: 0,
            borderLeft: '1px solid rgba(148, 163, 184, 0.15)',
            background: 'rgba(9, 11, 16, 0.85)',
            display: 'flex',
            flexDirection: 'column',
            minHeight: 0,
          }}
        >
          <Box
            sx={{
              px: 1.5,
              py: 0.75,
              borderBottom: '1px solid rgba(148, 163, 184, 0.15)',
              flexShrink: 0,
            }}
          >
            <Typography
              variant="caption"
              sx={{
                color: 'rgba(255,255,255,0.55)',
                textTransform: 'uppercase',
                letterSpacing: '0.08em',
              }}
            >
              Variables &amp; Call stack
            </Typography>
          </Box>
          <Box sx={{ flex: 1, minHeight: 0, overflow: 'hidden' }}>
            <VariablesPanel hit={pausedHit} />
          </Box>
        </Box>
      </Box>
    </Box>
  );
}

'use client';

import { cpp } from '@codemirror/lang-cpp';
import { oneDark } from '@codemirror/theme-one-dark';
import { Runtime } from '@jtrb/runtime';
import FlashOnIcon from '@mui/icons-material/FlashOn';
import PlayArrowIcon from '@mui/icons-material/PlayArrow';
import StopIcon from '@mui/icons-material/Stop';
import { Box, Button, FormControl, InputLabel, MenuItem, Select, Typography } from '@mui/material';
import CodeMirror from '@uiw/react-codemirror';
import React, { useEffect, useRef, useState } from 'react';

import Terminal, { TerminalHandle } from '@/components/Terminal';

const defaultCode = `#include <iostream>

int main() {
  int x;
  std::cin >> x;
  std::cout << x << std::endl;
  return 0;
}`;

type Language = 'C' | 'C++';

/** Which step of the manual API walkthrough the user has reached (resets when the run completes). */
type DemoStep = 'idle' | 'runtime' | 'wired' | 'init_sent';

export default function CodeEditor() {
  const [code, setCode] = useState<string>(defaultCode);
  const [isRunning, setIsRunning] = useState(false);
  const [isPaused, setIsPaused] = useState(false);
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
  const abortControllerRef = useRef<AbortController | null>(null);
  const [demoStep, setDemoStep] = useState<DemoStep>('idle');
  const dapSeqRef = useRef<number>(1);
  /** Line for setBreakpoints on the next Run, after the `initialized` event (null = none). */
  const pendingBreakpointLineRef = useRef<number | null>(null);

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
    // runtimeRef.current?.debugger.resume();
    setIsPaused(false);
  };

  const writeDemoLog = (message: string) => {
    terminalRef.current?.writeln(`[demo] ${message}`);
    console.log(`[demo] ${message}`);
  };

  /** Drop a partial walkthrough session so Quick run or step 1 can start clean. */
  const teardownDemoSession = () => {
    if (onDataHandlerRef.current) {
      onDataHandlerRef.current.dispose();
      onDataHandlerRef.current = null;
    }
    if (abortControllerRef.current) {
      abortControllerRef.current.abort();
      abortControllerRef.current = null;
    }
    runtimeRef.current = null;
    setDemoStep('idle');
  };

  const finalizeAfterRun = () => {
    if (onDataHandlerRef.current) {
      onDataHandlerRef.current.dispose();
      onDataHandlerRef.current = null;
    }
    if (abortControllerRef.current) {
      abortControllerRef.current.abort();
      abortControllerRef.current = null;
    }
    runtimeRef.current = null;
    isRunningRef.current = false;
    terminalRef.current?.disableInput();
    setIsRunning(false);
    setIsPaused(false);
    setDemoStep('idle');
  };

  /**
   * After the worker instruments the binary it blocks until configurationDone.
   * That request must run only once the debugger is attached — i.e. after the
   * `initialized` event. Sending configurationDone earlier is a no-op and the run hangs.
   */
  const completeDapAfterInitialized = async () => {
    const line = pendingBreakpointLineRef.current;
    const breakpoints = line != null ? [{ line }] : [];
    await sendDapRequest('setBreakpoints', {
      source: { path: '/main.c' },
      breakpoints,
    });
    await sendDapRequest('setExceptionBreakpoints', { filters: [] });
    await sendDapRequest('configurationDone', {});
  };

  const setupRuntimeForDemo = async () => {
    if (runtimeRef.current) {
      return runtimeRef.current;
    }

    const rt = await Runtime.create('c');
    runtimeRef.current = rt;
    (window as { __rt?: typeof rt }).__rt = rt;
    dapSeqRef.current = 1;

    rt.debugger.on('event', (msg: unknown) => {
      if (!msg || typeof msg !== 'object' || !('type' in msg)) {
        console.log('DAP EVENT (unknown payload):', msg);
        return;
      }
      const dapMsg = msg as { type: string; event?: string };
      console.log(dapMsg.type === 'event' ? 'DAP EVENT:' : 'DAP RESPONSE:', dapMsg);
      if (dapMsg.type === 'event' && dapMsg.event === 'initialized') {
        writeDemoLog('received initialized event — sending breakpoints + configurationDone');
        void completeDapAfterInitialized();
      }
    });

    writeDemoLog('runtime created and debugger ready');
    return rt;
  };

  const sendDapRequest = async (command: string, args: Record<string, unknown>) => {
    const rt = await setupRuntimeForDemo();
    const request = {
      type: 'request' as const,
      seq: dapSeqRef.current++,
      command,
      arguments: args,
    };
    console.log('DAP SEND:', request);
    writeDemoLog(`send ${command}`);
    const response = rt.debugger.send(request);
    console.log('DAP SYNC RESPONSE:', response);
    writeDemoLog(`response ${command}: ${JSON.stringify(response)}`);
    return response;
  };

  const handleSetBreakpoint = () => {
    pendingBreakpointLineRef.current = 4;
    writeDemoLog('queued breakpoint at line 4 (applied when the initialized handler runs).');
  };

  const handleClearBreakpoints = () => {
    pendingBreakpointLineRef.current = null;
    writeDemoLog('cleared queued breakpoints.');
  };

  /**
   * Wire stdout/stderr into the terminal, set VFS, stdin handling, and breakpoint UI.
   * Call this after Runtime.create and before initialize + run().
   */
  const wireExecutionEnvironment = async (rt: Runtime) => {
    if (abortControllerRef.current) {
      abortControllerRef.current.abort();
    }
    abortControllerRef.current = new AbortController();
    const signal = abortControllerRef.current.signal;

    const terminal = new WritableStream<Uint8Array>({
      write: (chunk) => {
        const decoder = new TextDecoder();
        const text = decoder.decode(chunk);
        const normalized = text.replace(/\r?\n/g, '\r\n');
        terminalRef.current?.write(normalized);
      },
      abort: () => {},
    });

    const stderrStream = new WritableStream<Uint8Array>({
      write: (chunk) => {
        const decoder = new TextDecoder();
        const text = decoder.decode(chunk);
        const normalized = text.replace(/\r?\n/g, '\r\n');
        terminalRef.current?.write(normalized);
      },
      abort: () => {},
    });

    rt.stdout.pipeTo(terminal, { signal }).catch(() => {});
    rt.stderr.pipeTo(stderrStream, { signal }).catch(() => {});
    rt.fs = { 'main.c': code };

    const term = terminalRef.current?.getTerminal();
    if (!term) {
      throw new Error('Terminal not initialized');
    }
    terminalRef.current?.enableInput();

    let stdinBuffer = '';
    const encoder = new TextEncoder();
    const stdinWriter = rt.stdin.getWriter();

    if (onDataHandlerRef.current) {
      onDataHandlerRef.current.dispose();
      onDataHandlerRef.current = null;
    }

    const onData = term.onData((data: string) => {
      if (!isRunningRef.current) return;

      if (data === '\x03') {
        term.write('^C\r\n');
        wasStoppedByUserRef.current = true;
        const r = runtimeRef.current as Runtime & { stop?: () => void };
        if (r?.stop) {
          r.stop();
        }
        isRunningRef.current = false;
        setIsRunning(false);
        terminalRef.current?.disableInput();
        terminalRef.current?.writeln('Execution stopped by user (Ctrl+C)');
        return;
      }
      if (data === '\x04') {
        term.write('^D\r\n');
        stdinWriter.write(encoder.encode('\x04'));
        stdinBuffer = '';
        return;
      }
      if (data === '\x0c') {
        terminalRef.current?.clear();
        stdinBuffer = '';
        return;
      }
      if (data === '\r') {
        term.write('\r\n');
        stdinWriter.write(encoder.encode(`${stdinBuffer}\n`));
        stdinBuffer = '';
      } else if (data === '\u007f') {
        if (stdinBuffer.length > 0) {
          stdinBuffer = stdinBuffer.slice(0, -1);
          term.write('\b \b');
        }
      } else if (data === '\x1b[A' || data === '\x1b[B' || data === '\x1b[C' || data === '\x1b[D') {
        return;
      } else {
        stdinBuffer += data;
        term.write(data);
      }
    });

    onDataHandlerRef.current = onData;
  };

  const handleStepCreateRuntime = async () => {
    try {
      terminalRef.current?.clear();
      terminalRef.current?.writeln("[demo] Step 1: Runtime.create('c')");
      dapSeqRef.current = 1;
      await setupRuntimeForDemo();
      setDemoStep('runtime');
      writeDemoLog('runtime ready — next: wire I/O and filesystem (step 2).');
    } catch (error) {
      console.error('Failed to create runtime:', error);
      writeDemoLog('failed to create runtime');
    }
  };

  const handleStepWireIo = async () => {
    const rt = runtimeRef.current;
    if (!rt || demoStep !== 'runtime') return;
    try {
      terminalRef.current?.writeln('[demo] Step 2: rt.stdout/stderr, rt.fs, stdin');
      await wireExecutionEnvironment(rt);
      setDemoStep('wired');
      writeDemoLog('I/O and fs wired — next: send initialize (step 3).');
    } catch (error) {
      console.error('Failed to wire I/O:', error);
      writeDemoLog('failed to wire I/O');
    }
  };

  const handleStepSendInitialize = async () => {
    if (demoStep !== 'wired') return;
    try {
      terminalRef.current?.writeln('[demo] Step 3: dbg.send(initialize)');
      await sendDapRequest('initialize', {});
      setDemoStep('init_sent');
      writeDemoLog('initialize OK — next: await rt.run() (step 4).');
    } catch (error) {
      console.error('Failed initialize:', error);
      writeDemoLog('initialize failed');
    }
  };

  const handleStepStartRun = async () => {
    const rt = runtimeRef.current;
    if (!rt || demoStep !== 'init_sent') return;

    setIsRunning(true);
    isRunningRef.current = true;
    wasStoppedByUserRef.current = false;

    try {
      terminalRef.current?.writeln(
        '[demo] Step 4: await rt.run() — worker starts; DAP completes on initialized'
      );
      terminalRef.current?.writeln('Running...');
      rt.fs = { 'main.c': code };
      await rt.run();
    } catch (error) {
      console.error('Failed to run code:', error);
      if (wasStoppedByUserRef.current) {
        return;
      }
      terminalRef.current?.writeln('Error: So much stuff will be here once the runtime is wired.');
    } finally {
      finalizeAfterRun();
    }
  };

  /**
   * One click: full pipeline so you can exercise stdout/stdin/etc. without stepping.
   * Still sends minimal DAP (initialize + post-initialized handshake) because debug builds block until configurationDone.
   */
  const handleQuickRun = async () => {
    if (isRunning || isPaused) return;
    teardownDemoSession();
    setIsRunning(true);
    isRunningRef.current = true;
    wasStoppedByUserRef.current = false;
    try {
      terminalRef.current?.clear();
      terminalRef.current?.writeln('[demo] Quick run: create → wire → initialize → run()');
      dapSeqRef.current = 1;
      const rt = await setupRuntimeForDemo();
      await wireExecutionEnvironment(rt);
      await sendDapRequest('initialize', {});
      terminalRef.current?.writeln('Running...');
      rt.fs = { 'main.c': code };
      await rt.run();
    } catch (error) {
      console.error('Quick run failed:', error);
      if (!wasStoppedByUserRef.current) {
        terminalRef.current?.writeln(
          'Error: So much stuff will be here once the runtime is wired.'
        );
      }
    } finally {
      finalizeAfterRun();
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

  const extensions = [cpp()];

  return (
    <Box
      sx={{
        height: '100%',
        minHeight: 0,
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden',
      }}
    >
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
          flexShrink: 0,
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
            <Typography
              variant="caption"
              sx={{ color: 'rgba(255, 255, 255, 0.5)', fontSize: '0.7rem' }}
            >
              Steps below · reference on the right
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
        <Box sx={{ display: 'flex', alignItems: 'center', gap: 1, flexWrap: 'wrap' }}>
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

          <Button
            size="small"
            variant="outlined"
            startIcon={<FlashOnIcon sx={{ fontSize: 18 }} />}
            onClick={handleQuickRun}
            disabled={isRunning || isPaused}
            sx={{
              textTransform: 'none',
              borderColor: 'rgba(250, 204, 21, 0.45)',
              color: '#fcd34d',
            }}
          >
            Quick run
          </Button>

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
        </Box>
      </Box>
      <Box sx={{ display: 'flex', flex: 1, minHeight: 0, overflow: 'hidden' }}>
        <Box
          sx={{
            flex: 1,
            minWidth: 0,
            minHeight: 0,
            display: 'flex',
            flexDirection: 'column',
            overflow: 'hidden',
          }}
        >
          <Box
            sx={{
              px: 2,
              py: 0.75,
              borderBottom: '1px solid',
              borderColor: 'rgba(148, 163, 184, 0.12)',
              display: 'flex',
              alignItems: 'center',
              gap: 0.75,
              flexWrap: 'wrap',
              flexShrink: 0,
              background: 'rgba(8, 10, 15, 0.35)',
            }}
          >
            <Button
              size="small"
              variant="outlined"
              onClick={handleStepCreateRuntime}
              disabled={isRunning || isPaused || demoStep !== 'idle'}
              sx={{ textTransform: 'none', fontSize: '0.7rem', py: 0.25, minWidth: 0 }}
            >
              1 Create
            </Button>
            <Button
              size="small"
              variant="outlined"
              onClick={handleStepWireIo}
              disabled={isRunning || isPaused || demoStep !== 'runtime'}
              sx={{ textTransform: 'none', fontSize: '0.7rem', py: 0.25, minWidth: 0 }}
            >
              2 Wire
            </Button>
            <Button
              size="small"
              variant="outlined"
              onClick={handleStepSendInitialize}
              disabled={isRunning || isPaused || demoStep !== 'wired'}
              sx={{ textTransform: 'none', fontSize: '0.7rem', py: 0.25, minWidth: 0 }}
            >
              3 Init
            </Button>
            <Button
              size="small"
              variant="contained"
              startIcon={<PlayArrowIcon sx={{ fontSize: 16 }} />}
              onClick={handleStepStartRun}
              disabled={isRunning || isPaused || demoStep !== 'init_sent'}
              sx={{
                textTransform: 'none',
                fontSize: '0.7rem',
                py: 0.25,
                minWidth: 0,
                background: 'linear-gradient(135deg, #6366f1 0%, #7c3aed 100%)',
                '&:hover': { background: 'linear-gradient(135deg, #5855eb 0%, #6d28d9 100%)' },
              }}
            >
              4 Run
            </Button>
            <Button
              size="small"
              variant="outlined"
              color="error"
              startIcon={<StopIcon sx={{ fontSize: 16 }} />}
              onClick={handleStop}
              disabled={!isRunning}
              sx={{ textTransform: 'none', fontSize: '0.7rem', py: 0.25, minWidth: 0 }}
            >
              Stop
            </Button>
            <Button
              size="small"
              variant="text"
              onClick={handleSetBreakpoint}
              disabled={isRunning}
              sx={{ fontSize: '0.68rem' }}
            >
              BP@4
            </Button>
            <Button
              size="small"
              variant="text"
              onClick={handleClearBreakpoints}
              disabled={isRunning}
              sx={{ fontSize: '0.68rem' }}
            >
              Clear BP
            </Button>
          </Box>
          <Box
            ref={containerRef}
            sx={{
              flex: 1,
              minHeight: 0,
              overflow: 'hidden',
              display: 'flex',
              flexDirection: 'column',
              position: 'relative',
            }}
          >
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
                  lineNumbers: true,
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
            </Box>

            {/* Terminal - fixed height, resizable */}
            <Terminal ref={terminalRef} height={terminalHeight} />
          </Box>
        </Box>
        <Box
          sx={{
            width: { xs: 0, sm: 200, md: 220 },
            display: { xs: 'none', sm: 'flex' },
            flexDirection: 'column',
            flexShrink: 0,
            borderLeft: '1px solid rgba(148, 163, 184, 0.12)',
            background: 'rgba(6, 8, 12, 0.55)',
            overflow: 'hidden',
          }}
        >
          <Typography
            sx={{
              px: 1.25,
              py: 0.75,
              fontSize: '0.62rem',
              fontWeight: 600,
              letterSpacing: '0.06em',
              textTransform: 'uppercase',
              color: 'rgba(255,255,255,0.45)',
              borderBottom: '1px solid rgba(148, 163, 184, 0.1)',
            }}
          >
            API order
          </Typography>
          <Box
            component="ol"
            sx={{
              m: 0,
              py: 0.75,
              px: 1.5,
              pl: 2,
              overflowY: 'auto',
              flex: 1,
              minHeight: 0,
              color: 'rgba(255,255,255,0.42)',
              fontSize: '0.62rem',
              lineHeight: 1.45,
              '& code': { fontSize: '0.58rem', color: '#a5b4fc' },
            }}
          >
            <Box component="li" sx={{ mb: 0.5 }}>
              <code>Runtime.create</code> — listener on <code>initialized</code> sends breakpoints +{' '}
              <code>configurationDone</code>.
            </Box>
            <Box component="li" sx={{ mb: 0.5 }}>
              Wire streams + <code>rt.fs</code> (<code>main.c</code>).
            </Box>
            <Box component="li" sx={{ mb: 0.5 }}>
              <code>initialize</code> (capabilities).
            </Box>
            <Box component="li" sx={{ mb: 0.5 }}>
              <code>await rt.run()</code> — worker blocks until DAP config completes.
            </Box>
          </Box>
          <Typography
            sx={{
              px: 1.25,
              py: 0.6,
              fontSize: '0.58rem',
              color: 'rgba(255,255,255,0.35)',
              lineHeight: 1.35,
            }}
          >
            Quick run still does minimal DAP (debug build); use it to test I/O without clicking 1–4.
          </Typography>
        </Box>
      </Box>
    </Box>
  );
}

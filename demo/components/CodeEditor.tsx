'use client';

import { cpp } from '@codemirror/lang-cpp';
import { oneDark } from '@codemirror/theme-one-dark';
import { EditorView, gutter, GutterMarker } from '@codemirror/view';
import { Runtime } from '@jtrb/runtime';
import PlayArrowIcon from '@mui/icons-material/PlayArrow';
import SkipNextIcon from '@mui/icons-material/SkipNext';
import StopIcon from '@mui/icons-material/Stop';
import SubdirectoryArrowLeftIcon from '@mui/icons-material/SubdirectoryArrowLeft';
import SubdirectoryArrowRightIcon from '@mui/icons-material/SubdirectoryArrowRight';
import { Box, Button, FormControl, InputLabel, MenuItem, Select, Typography } from '@mui/material';
import CodeMirror from '@uiw/react-codemirror';
import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import Terminal, { TerminalHandle } from '@/components/Terminal';
import VariablesPanel, {
  type ScopeBlock,
  type StackFrameRow,
  type VarNode
} from '@/components/VariablesPanel';

const defaultCode = `#include <iostream>

int main() {
  int x;
  std::cin >> x;
  std::cout << x << std::endl;
  return 0;
}`;

type Language = 'C' | 'C++';

/** Left gutter: click to toggle a DAP line breakpoint (1-based line numbers). */
function breakpointGutterExtension(
  lines: readonly number[],
  gutterDisabled: boolean,
  onToggle: (line: number) => void
) {
  const lineSet = new Set(lines);

  class BreakpointDot extends GutterMarker {
    elementClass = 'cm-demo-bp-marker';

    eq(other: GutterMarker): boolean {
      return other instanceof BreakpointDot;
    }

    toDOM(): Node {
      const el = document.createElement('div');
      el.textContent = '●';
      el.setAttribute('aria-label', 'Remove breakpoint');
      return el;
    }
  }

  return [
    gutter({
      class: 'cm-demo-bp-gutter',
      renderEmptyElements: true,
      lineMarker(view, block) {
        const lineNo = view.state.doc.lineAt(block.from).number;
        return lineSet.has(lineNo) ? new BreakpointDot() : null;
      },
      domEventHandlers: {
        mousedown(view, block, event) {
          if (gutterDisabled) return false;
          const e = event as MouseEvent;
          if (e.button !== 0) return false;
          const lineNo = view.state.doc.lineAt(block.from).number;
          onToggle(lineNo);
          return true;
        }
      }
    }),
    EditorView.baseTheme({
      '.cm-demo-bp-gutter': {
        width: '16px',
        minWidth: '16px',
        cursor: gutterDisabled ? 'default' : 'pointer'
      },
      '.cm-demo-bp-gutter .cm-gutterElement': {
        display: 'flex',
        alignItems: 'flex-start',
        justifyContent: 'center',
        padding: '1px 2px 0',
        color: '#f87171',
        fontSize: '9px',
        lineHeight: '1.45'
      },
      '.cm-demo-bp-gutter .cm-gutterElement:hover': {
        backgroundColor: gutterDisabled ? 'transparent' : 'rgba(248, 113, 113, 0.12)'
      }
    })
  ];
}

function unwrapDapBody<T>(response: unknown): T | null {
  if (!response || typeof response !== 'object') return null;
  const r = response as { success?: boolean; body?: T; message?: string };
  if (!r.success) {
    console.warn('DAP request failed:', r.message);
    return null;
  }
  return r.body ?? null;
}

function parseVariable(v: unknown): VarNode {
  const o = v as Record<string, unknown>;
  const ref = o.variablesReference;
  return {
    name: String(o.name ?? ''),
    value: String(o.value ?? ''),
    type: o.type != null ? String(o.type) : undefined,
    variablesReference: typeof ref === 'number' ? ref : 0
  };
}

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
  /** Removes stdout/stderr listeners from the last wire call. */
  const ioCleanupRef = useRef<(() => void) | null>(null);
  const dapSeqRef = useRef<number>(1);
  /** 1-based source lines with breakpoints; sent in `setBreakpoints` after `initialized`. */
  const [breakpointLines, setBreakpointLines] = useState<number[]>([]);
  const breakpointLinesRef = useRef<number[]>([]);
  breakpointLinesRef.current = breakpointLines;

  const [debugFrames, setDebugFrames] = useState<StackFrameRow[]>([]);
  const [selectedFrameId, setSelectedFrameId] = useState(0);
  const [scopeBlocks, setScopeBlocks] = useState<ScopeBlock[]>([]);
  const [varChildMap, setVarChildMap] = useState<Record<number, VarNode[]>>({});
  const varChildMapRef = useRef<Record<number, VarNode[]>>({});
  varChildMapRef.current = varChildMap;
  const [debugLoading, setDebugLoading] = useState(false);

  const toggleBreakpointLine = useCallback((line: number) => {
    setBreakpointLines((prev) => {
      const next = new Set(prev);
      if (next.has(line)) next.delete(line);
      else next.add(line);
      return [...next].sort((a, b) => a - b);
    });
  }, []);

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

  const dapSend = useCallback((rt: Runtime, command: string, args: Record<string, unknown>) => {
    const request = {
      type: 'request' as const,
      seq: dapSeqRef.current++,
      command,
      arguments: args
    };
    return rt.debugger.send(request);
  }, []);

  const loadScopesForFrame = useCallback(
    async (rt: Runtime, frameId: number) => {
      const scBody = unwrapDapBody<{ scopes: { name: string; variablesReference: number }[] }>(
        dapSend(rt, 'scopes', { frameId })
      );
      const scopesMeta = scBody?.scopes ?? [];
      const blocks: ScopeBlock[] = [];
      for (const s of scopesMeta) {
        const vBody = unwrapDapBody<{ variables: unknown[] }>(
          dapSend(rt, 'variables', { variablesReference: s.variablesReference })
        );
        blocks.push({
          name: s.name,
          variables: (vBody?.variables ?? []).map(parseVariable)
        });
      }
      setScopeBlocks(blocks);
    },
    [dapSend]
  );

  const refreshDebugSession = useCallback(
    async (rt: Runtime) => {
      setDebugLoading(true);
      try {
        const st = unwrapDapBody<{
          stackFrames: {
            id: number;
            name: string;
            line?: number;
            source?: { path?: string };
          }[];
        }>(dapSend(rt, 'stackTrace', { threadId: 1 }));
        const rawFrames = st?.stackFrames ?? [];
        const mapped: StackFrameRow[] = rawFrames.map((f) => ({
          id: f.id,
          name: f.name,
          line: f.line,
          path: f.source?.path
        }));
        setDebugFrames(mapped);
        const frameId = mapped[0]?.id ?? 0;
        setSelectedFrameId(frameId);
        setVarChildMap({});
        await loadScopesForFrame(rt, frameId);
      } finally {
        setDebugLoading(false);
      }
    },
    [loadScopesForFrame, dapSend]
  );

  const selectFrame = useCallback(
    async (frameId: number) => {
      const rt = runtimeRef.current;
      if (!rt) return;
      setSelectedFrameId(frameId);
      setDebugLoading(true);
      setVarChildMap({});
      try {
        await loadScopesForFrame(rt, frameId);
      } finally {
        setDebugLoading(false);
      }
    },
    [loadScopesForFrame]
  );

  const handleExpandVariable = useCallback(
    async (variablesReference: number) => {
      const rt = runtimeRef.current;
      if (!rt || variablesReference <= 0) return;
      if (varChildMapRef.current[variablesReference] !== undefined) return;
      const body = unwrapDapBody<{ variables: unknown[] }>(
        dapSend(rt, 'variables', { variablesReference })
      );
      const next = (body?.variables ?? []).map(parseVariable);
      setVarChildMap((m) => ({ ...m, [variablesReference]: next }));
    },
    [dapSend]
  );

  const handleContinue = () => {
    const rt = runtimeRef.current;
    if (!rt) return;
    dapSend(rt, 'continue', { threadId: 1 });
    setIsPaused(false);
    setDebugFrames([]);
    setScopeBlocks([]);
    setVarChildMap({});
  };

  const dispatchStepCommand = useCallback(
    (command: 'next' | 'stepIn' | 'stepOut') => {
      const rt = runtimeRef.current;
      if (!rt) return;
      setDebugLoading(true);
      setVarChildMap({});
      setScopeBlocks([]);
      setDebugFrames([]);
      dapSend(rt, command, { threadId: 1 });
    },
    [dapSend]
  );

  /** Drop I/O and runtime so a new run can start clean. */
  const teardownDemoSession = () => {
    if (onDataHandlerRef.current) {
      onDataHandlerRef.current.dispose();
      onDataHandlerRef.current = null;
    }
    if (ioCleanupRef.current) {
      ioCleanupRef.current();
      ioCleanupRef.current = null;
    }
    runtimeRef.current = null;
  };

  const finalizeAfterRun = () => {
    if (onDataHandlerRef.current) {
      onDataHandlerRef.current.dispose();
      onDataHandlerRef.current = null;
    }
    if (ioCleanupRef.current) {
      ioCleanupRef.current();
      ioCleanupRef.current = null;
    }
    runtimeRef.current = null;
    isRunningRef.current = false;
    terminalRef.current?.disableInput();
    setIsRunning(false);
    setIsPaused(false);
    setDebugFrames([]);
    setScopeBlocks([]);
    setVarChildMap({});
    setDebugLoading(false);
  };

  /**
   * After the worker instruments the binary it blocks until configurationDone.
   * That request must run only once the debugger is attached — i.e. after the
   * `initialized` event. Sending configurationDone earlier is a no-op and the run hangs.
   */
  const completeDapAfterInitialized = () => {
    const rt = runtimeRef.current;
    if (!rt) return;
    const breakpoints = breakpointLinesRef.current.map((line) => ({ line }));
    dapSend(rt, 'setBreakpoints', {
      source: { path: '/main.c' },
      breakpoints
    });
    dapSend(rt, 'setExceptionBreakpoints', { filters: [] });
    dapSend(rt, 'configurationDone', {});
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
        return;
      }
      const dapMsg = msg as {
        type: string;
        event?: string;
        body?: { reason?: string; threadId?: number };
      };
      if (dapMsg.type === 'event' && dapMsg.event === 'initialized') {
        completeDapAfterInitialized();
        return;
      }
      if (dapMsg.type === 'event' && dapMsg.event === 'stopped') {
        setIsPaused(true);
        void refreshDebugSession(rt);
        return;
      }
      if (dapMsg.type === 'event' && dapMsg.event === 'terminated') {
        setIsPaused(false);
        setDebugFrames([]);
        setScopeBlocks([]);
        setVarChildMap({});
        setDebugLoading(false);
      }
    });

    return rt;
  };

  const handleClearBreakpoints = () => {
    setBreakpointLines([]);
  };

  /**
   * Wire stdout/stderr into the terminal, set VFS, stdin handling, and breakpoint UI.
   * Call this after Runtime.create and before initialize + run().
   */
  const wireExecutionEnvironment = async (rt: Runtime) => {
    ioCleanupRef.current?.();
    const decoder = new TextDecoder();
    const onTermChunk = (chunk: Uint8Array) => {
      const text = decoder.decode(chunk);
      const normalized = text.replace(/\r?\n/g, '\r\n');
      terminalRef.current?.write(normalized);
    };
    rt.stdout.on('data', onTermChunk);
    rt.stderr.on('data', onTermChunk);
    ioCleanupRef.current = () => {
      rt.stdout.off('data', onTermChunk);
      rt.stderr.off('data', onTermChunk);
    };
    rt.fs = { 'main.c': code };

    const term = terminalRef.current?.getTerminal();
    if (!term) {
      throw new Error('Terminal not initialized');
    }
    terminalRef.current?.enableInput();

    let stdinBuffer = '';
    const encoder = new TextEncoder();

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
        void rt.stdin.write(encoder.encode('\x04'));
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
        void rt.stdin.write(encoder.encode(`${stdinBuffer}\n`));
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

  /** Create runtime, wire terminal/fs/stdin, DAP initialize, then run (debug build). */
  const handleRun = async () => {
    if (isRunning) return;
    teardownDemoSession();
    setIsRunning(true);
    isRunningRef.current = true;
    wasStoppedByUserRef.current = false;
    try {
      terminalRef.current?.clear();
      dapSeqRef.current = 1;
      const rt = await setupRuntimeForDemo();
      await wireExecutionEnvironment(rt);
      dapSend(rt, 'initialize', {});
      terminalRef.current?.writeln('Running...');
      rt.fs = { 'main.c': code };
      await rt.run();
    } catch (error) {
      console.error('Run failed:', error);
      if (!wasStoppedByUserRef.current) {
        terminalRef.current?.writeln(
          `Error: ${error instanceof Error ? error.message : String(error)}`
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

  const extensions = useMemo(
    () => [
      breakpointGutterExtension(breakpointLines, isRunning && !isPaused, toggleBreakpointLine),
      cpp()
    ],
    [breakpointLines, isRunning, isPaused, toggleBreakpointLine]
  );

  return (
    <Box
      sx={{
        height: '100%',
        minHeight: 0,
        display: 'flex',
        flexDirection: 'column',
        overflow: 'hidden'
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
          backdropFilter: 'blur(8px)'
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
                  border: '1px solid rgba(99, 102, 241, 0.45)'
                }}
              >
                Demo
              </Box>
            </Box>
            <Typography
              variant="caption"
              sx={{ color: 'rgba(255, 255, 255, 0.5)', fontSize: '0.7rem' }}
            >
              Editor · terminal · stack & variables
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
                  borderColor: 'rgba(148, 163, 184, 0.35)'
                },
                '&:hover .MuiOutlinedInput-notchedOutline': {
                  borderColor: 'rgba(148, 163, 184, 0.55)'
                }
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
              fontVariantNumeric: 'tabular-nums'
            }}
          >
            {code.split('\n').length} lines
          </Typography>

          <Button
            size="small"
            variant="contained"
            startIcon={<PlayArrowIcon sx={{ fontSize: 18 }} />}
            onClick={handleRun}
            disabled={isRunning}
            sx={{
              textTransform: 'none',
              background: 'linear-gradient(135deg, #6366f1 0%, #7c3aed 100%)',
              '&:hover': { background: 'linear-gradient(135deg, #5855eb 0%, #6d28d9 100%)' }
            }}
          >
            Run
          </Button>

          <Button
            size="small"
            variant="outlined"
            color="error"
            startIcon={<StopIcon sx={{ fontSize: 16 }} />}
            onClick={handleStop}
            disabled={!isRunning}
            sx={{ textTransform: 'none' }}
          >
            Stop
          </Button>

          <Button
            size="small"
            variant="text"
            onClick={handleClearBreakpoints}
            disabled={isRunning && !isPaused}
            sx={{ fontSize: '0.75rem', textTransform: 'none' }}
          >
            Clear breakpoints
          </Button>

          {isPaused && (
            <>
              <Button
                variant="contained"
                size="small"
                startIcon={<PlayArrowIcon />}
                onClick={handleContinue}
                sx={{
                  minWidth: 96,
                  textTransform: 'none',
                  background: 'linear-gradient(135deg, #22c55e 0%, #16a34a 100%)',
                  border: '1px solid rgba(255, 255, 255, 0.12)',
                  '&:hover': {
                    background: 'linear-gradient(135deg, #16a34a 0%, #15803d 100%)'
                  }
                }}
              >
                Continue
              </Button>
              <Button
                variant="outlined"
                size="small"
                startIcon={<SkipNextIcon sx={{ fontSize: 16 }} />}
                onClick={() => dispatchStepCommand('next')}
                sx={{ textTransform: 'none', borderColor: 'rgba(148, 163, 184, 0.45)' }}
              >
                Step over
              </Button>
              <Button
                variant="outlined"
                size="small"
                startIcon={<SubdirectoryArrowRightIcon sx={{ fontSize: 16 }} />}
                onClick={() => dispatchStepCommand('stepIn')}
                sx={{ textTransform: 'none', borderColor: 'rgba(148, 163, 184, 0.45)' }}
              >
                Step into
              </Button>
              <Button
                variant="outlined"
                size="small"
                startIcon={<SubdirectoryArrowLeftIcon sx={{ fontSize: 16 }} />}
                onClick={() => dispatchStepCommand('stepOut')}
                sx={{ textTransform: 'none', borderColor: 'rgba(148, 163, 184, 0.45)' }}
              >
                Step out
              </Button>
            </>
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
            overflow: 'hidden'
          }}
        >
          <Box
            ref={containerRef}
            sx={{
              flex: 1,
              minHeight: 0,
              overflow: 'hidden',
              display: 'flex',
              flexDirection: 'column',
              position: 'relative'
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
                  highlightSelectionMatches: true
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
                  backgroundColor: 'rgba(148, 163, 184, 0.3)'
                },
                '&::before': {
                  content: '""',
                  position: 'absolute',
                  top: '-2px',
                  left: 0,
                  right: 0,
                  height: '8px',
                  cursor: 'row-resize'
                }
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
                flexShrink: 0
              }}
            >
              <Typography
                variant="caption"
                sx={{
                  color: 'rgba(255, 255, 255, 0.55)',
                  letterSpacing: '0.12em',
                  textTransform: 'uppercase'
                }}
              >
                Output
              </Typography>
            </Box>

            {/* Terminal - fixed height, resizable */}
            <Terminal ref={terminalRef} height={terminalHeight} />
          </Box>
        </Box>
        <VariablesPanel
          isPaused={isPaused}
          debugLoading={debugLoading}
          frames={debugFrames}
          selectedFrameId={selectedFrameId}
          onSelectFrame={(id) => void selectFrame(id)}
          scopes={scopeBlocks}
          varChildMap={varChildMap}
          onExpandVariable={(ref) => void handleExpandVariable(ref)}
        />
      </Box>
    </Box>
  );
}

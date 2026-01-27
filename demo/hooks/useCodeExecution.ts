'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { Runtime } from 'runtime';

import type { TerminalHandle } from '@/components/Terminal';

type UseCodeExecutionOptions = {
  /** Reference to the terminal component */
  terminalRef: React.RefObject<TerminalHandle | null>;
  /** Whether the terminal is ready (xterm initialized) */
  terminalReady: boolean;
};

type UseCodeExecutionResult = {
  /** Whether code is currently running */
  isRunning: boolean;
  /** Run the provided code */
  runCode: (code: string) => Promise<void>;
  /** Stop the currently running code */
  stopCode: () => void;
};

/**
 * Hook to manage code execution with terminal I/O integration using streams.
 *
 * Handles:
 * - Running code via the Runtime
 * - Piping stdout/stderr streams to the terminal
 * - Capturing stdin from terminal input via stream writer
 * - Ctrl+C to stop execution
 * - Ctrl+D for EOF
 * - Ctrl+L to clear terminal
 */
export function useCodeExecution({
  terminalRef,
  terminalReady,
}: UseCodeExecutionOptions): UseCodeExecutionResult {
  const [isRunning, setIsRunning] = useState(false);
  const runtimeRef = useRef<Runtime | null>(null);
  const stdinWriterRef = useRef<WritableStreamDefaultWriter<Uint8Array> | null>(null);
  const isRunningRef = useRef(false);
  const encoderRef = useRef(new TextEncoder());

  // Set up persistent stdin handler (only when terminal is ready)
  useEffect(() => {
    if (!terminalReady) return;

    const terminal = terminalRef.current?.getTerminal();
    if (!terminal) return;

    let stdinBuffer = '';

    const onData = terminal.onData((data: string) => {
      // Only accept input when code is running
      if (!isRunningRef.current || !stdinWriterRef.current) return;

      const encoder = encoderRef.current;
      const writer = stdinWriterRef.current;

      // Control sequences
      if (data === '\x03') {
        // Ctrl+C: Stop execution
        terminal.write('^C\r\n');
        runtimeRef.current?.stop();
        return;
      } else if (data === '\x04') {
        // Ctrl+D: Send EOF
        terminal.write('^D\r\n');
        writer.write(encoder.encode('\x04'));
        stdinBuffer = '';
        return;
      } else if (data === '\x0c') {
        // Ctrl+L: Clear terminal
        terminalRef.current?.clear();
        stdinBuffer = '';
        return;
      }

      // Regular input handling
      if (data === '\r') {
        // Enter: send buffered input with newline
        terminal.write('\r\n');
        writer.write(encoder.encode(`${stdinBuffer}\n`));
        stdinBuffer = '';
      } else if (data === '\u007f') {
        // Backspace: remove last character
        if (stdinBuffer.length > 0) {
          stdinBuffer = stdinBuffer.slice(0, -1);
          terminal.write('\b \b');
        }
      } else if (
        // Ignore arrow keys
        data === '\x1b[A' ||
        data === '\x1b[B' ||
        data === '\x1b[C' ||
        data === '\x1b[D'
      ) {
        return;
      } else {
        // Regular character: add to buffer and echo
        stdinBuffer += data;
        terminal.write(data);
      }
    });

    return () => {
      onData.dispose();
    };
  }, [terminalRef, terminalReady]);

  const runCode = useCallback(
    async (code: string) => {
      if (isRunningRef.current) return; // Already running

      setIsRunning(true);
      isRunningRef.current = true;

      // Clear terminal for fresh output
      terminalRef.current?.clear();

      const rt = Runtime.create('c');
      runtimeRef.current = rt;
      rt.fs = { 'main.c': code };

      // Get stdin writer
      const writer = rt.stdin.getWriter();
      stdinWriterRef.current = writer;

      // Create a writable stream that writes to the terminal
      const decoder = new TextDecoder();
      const createTerminalWritable = () =>
        new WritableStream<Uint8Array>({
          write: (chunk) => {
            const text = decoder.decode(chunk);
            // Normalize newlines for terminal display
            const normalized = text.replace(/\r?\n/g, '\r\n');
            terminalRef.current?.write(normalized);
          },
        });

      // Create abort controller for stream cleanup
      const abortController = new AbortController();

      // Enable terminal input
      terminalRef.current?.enableInput();

      try {
        // Pipe stdout and stderr to terminal (non-blocking)
        const stdoutPipe = rt.stdout.pipeTo(createTerminalWritable(), {
          signal: abortController.signal,
        });
        const stderrPipe = rt.stderr.pipeTo(createTerminalWritable(), {
          signal: abortController.signal,
        });

        // Run the code
        await rt.run();

        // Abort the pipes after run completes
        abortController.abort();

        // Wait for pipes to finish (they'll reject due to abort, which is fine)
        await Promise.allSettled([stdoutPipe, stderrPipe]);
      } catch (error) {
        console.error('Execution error:', error);
        terminalRef.current?.writeln(
          `\r\nError: ${error instanceof Error ? error.message : String(error)}`
        );
      } finally {
        // Release the writer
        try {
          writer.releaseLock();
        } catch {
          // Ignore if already released
        }
        stdinWriterRef.current = null;
        runtimeRef.current = null;
        isRunningRef.current = false;
        setIsRunning(false);
        terminalRef.current?.disableInput();
      }
    },
    [terminalRef]
  );

  const stopCode = useCallback(() => {
    runtimeRef.current?.stop();
  }, []);

  return {
    isRunning,
    runCode,
    stopCode,
  };
}

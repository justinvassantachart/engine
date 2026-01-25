'use client';

import { cpp } from '@codemirror/lang-cpp';
import { oneDark } from '@codemirror/theme-one-dark';
import PlayArrowIcon from '@mui/icons-material/PlayArrow';
import { Box, Button, Typography } from '@mui/material';
import CodeMirror from '@uiw/react-codemirror';
import { useRef, useState } from 'react';
import { Runtime } from 'runtime';

import Terminal, { TerminalHandle } from '@/components/Terminal';

const defaultCode = `#include <iostream>

int main() {
  int x;
  std::cin >> x;
  std::cout << x << std::endl;
  return 0;
}`;

export default function CodeEditor() {
  const [code, setCode] = useState<string>(defaultCode);
  const [isRunning, setIsRunning] = useState<boolean>(false);
  const terminalRef = useRef<TerminalHandle | null>(null);

  /**
   * Handle Run Button Click
   *
   * TODO: Integrate with backend compiler/runtime
   *
   * This function should:
   * 1. Send the code and language to your backend API endpoint
   * 2. The backend should compile/interpret the code based on the language
   * 3. Execute the code in a secure sandboxed environment
   * 4. Return the output (stdout, stderr) or errors
   *
   * Example API call structure:
   * POST /api/run-code
   * Body: { code: string, language: 'javascript' | 'python' }
   * Response: { output: string, error?: string, exitCode: number }
   *
   * After receiving the response, you can:
   * - Display output in a console/output panel
   * - Show errors if compilation/runtime fails
   * - Update UI state based on execution status
   */
  const handleRun = async () => {
    setIsRunning(true);

    try {
      terminalRef.current?.clear();
      terminalRef.current?.writeln('Running...');

      // TODO: Replace this placeholder with runtime/compiler integration.
      // This is where we will:
      // 1) Initialize the runtime (or a worker) with the chosen language.
      // 2) Stream stdout/stderr into the terminal as the program runs.
      // 3) Surface compile/runtime errors with clear messaging.
      // TODO: When the runtime package is wired into the demo, replace this with:

      const rt = Runtime.create('c');

      const terminal = () =>
        new WritableStream<Uint8Array>({
          write: (chunk) => terminalRef.current?.write(chunk),
        });

      rt.stdout.pipeTo(terminal());
      rt.stderr.pipeTo(terminal());
      rt.fs = { 'main.c': code };

      const encoder = new TextEncoder();
      const writer = rt.stdin.getWriter();
      writer.write(encoder.encode('hello world\n'));

      await rt.run();
    } catch (error) {
      console.error('Failed to run code:', error);
      terminalRef.current?.writeln('Error: Failed to run code.');
    } finally {
      setIsRunning(false);
    }
  };

  const extensions = [cpp()];

  return (
    <Box sx={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <Box
        sx={{
          px: 3,
          py: 1.5,
          borderBottom: '1px solid',
          borderColor: 'rgba(255, 255, 255, 0.08)',
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          background: 'rgba(0, 0, 0, 0.2)',
          gap: 2,
        }}
      >
        <Box
          sx={{
            display: 'flex',
            alignItems: 'center',
            gap: 2,
            flex: 1,
            justifyContent: 'flex-end',
          }}
        >
          <Typography
            variant="caption"
            sx={{ color: 'rgba(255, 255, 255, 0.5)', fontSize: '0.75rem' }}
          >
            {code.split('\n').length} lines
          </Typography>

          {/* Run Button - Ready for backend integration */}
          <Button
            variant="contained"
            size="small"
            startIcon={<PlayArrowIcon />}
            onClick={handleRun}
            disabled={isRunning}
            sx={{
              minWidth: 100,
              textTransform: 'none',
              background: 'linear-gradient(135deg, #6366f1 0%, #8b5cf6 100%)',
              '&:hover': {
                background: 'linear-gradient(135deg, #5855eb 0%, #7c3aed 100%)',
              },
              '&:disabled': {
                background: 'rgba(99, 102, 241, 0.3)',
              },
            }}
          >
            {isRunning ? 'Running...' : 'Run'}
          </Button>
        </Box>
      </Box>
      <Box sx={{ flex: 1, overflow: 'hidden', display: 'flex', flexDirection: 'column' }}>
        <Box sx={{ flex: 1, overflow: 'hidden' }}>
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
        <Terminal ref={terminalRef} />
      </Box>
    </Box>
  );
}

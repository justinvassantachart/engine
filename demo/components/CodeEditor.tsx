'use client';

import { javascript } from '@codemirror/lang-javascript';
import { python } from '@codemirror/lang-python';
import { oneDark } from '@codemirror/theme-one-dark';
import PlayArrowIcon from '@mui/icons-material/PlayArrow';
import { Box, Button, FormControl, InputLabel, MenuItem, Select, Typography } from '@mui/material';
import CodeMirror from '@uiw/react-codemirror';
import { useState } from 'react';
import { Runtime } from 'runtime';

type Language = 'javascript' | 'python';

const defaultCode = {
  javascript: `// Welcome to the Code Editor
function greet(name) {
  return \`Hello, \${name}!\`;
}

console.log(greet('World'));`,
  python: `# Welcome to the Code Editor
def greet(name):
    return f"Hello, {name}!"

print(greet('World'))`,
};

export default function CodeEditor() {
  const [code, setCode] = useState<string>(defaultCode.javascript);
  const [language, setLanguage] = useState<Language>('javascript');
  const [isRunning, setIsRunning] = useState<boolean>(false);

  const handleLanguageChange = (newLanguage: Language) => {
    setLanguage(newLanguage);
    setCode(defaultCode[newLanguage]);
  };

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
      await Runtime.create('c');

      // Placeholder: Simulate API call
      await new Promise((resolve) => setTimeout(resolve, 500));
      console.log('Code to run:', { code, language });
    } catch (error) {
      console.error('Failed to run code:', error);
      // TODO: Display error message to user
    } finally {
      setIsRunning(false);
    }
  };

  const extensions = language === 'javascript' ? [javascript({ jsx: true })] : [python()];

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
        <FormControl size="small" sx={{ minWidth: 140 }}>
          <InputLabel sx={{ fontSize: '0.875rem' }}>Language</InputLabel>
          <Select
            value={language}
            label="Language"
            onChange={(e) => handleLanguageChange(e.target.value as Language)}
            sx={{
              fontSize: '0.875rem',
              '& .MuiOutlinedInput-notchedOutline': {
                borderColor: 'rgba(255, 255, 255, 0.12)',
              },
              '&:hover .MuiOutlinedInput-notchedOutline': {
                borderColor: 'rgba(255, 255, 255, 0.2)',
              },
            }}
          >
            <MenuItem value="javascript">JavaScript</MenuItem>
            <MenuItem value="python">Python</MenuItem>
          </Select>
        </FormControl>

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
    </Box>
  );
}

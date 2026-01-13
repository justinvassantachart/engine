# Code Editor UI

A minimal code editor UI built with Next.js, Material UI, and CodeMirror for testing coding/debugging tools.

## Features

- Modern code editor powered by CodeMirror
- Dark theme with Material UI
- Support for multiple languages (JavaScript, Python)
- Syntax highlighting
- Line numbers
- Code folding
- Auto-completion
- Bracket matching

## Getting Started

First, install the dependencies:

```bash
npm install
```

Then, run the development server:

```bash
npm run dev
```

Open [http://localhost:3000](http://localhost:3000) with your browser to see the result.

## Tech Stack

- **Next.js 14** - React framework
- **Material UI** - UI component library
- **CodeMirror** - Code editor component
- **TypeScript** - Type safety

## Project Structure

```
├── app/
│   ├── layout.tsx      # Root layout
│   ├── page.tsx        # Main page
│   └── globals.css     # Global styles
├── components/
│   └── CodeEditor.tsx  # Code editor component
└── package.json
```

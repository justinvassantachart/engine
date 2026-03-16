'use client';

import { useCallback, useRef, useState } from 'react';
import { Runtime } from 'runtime';

import { BENCHMARKS } from '@/config/benchmarks';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

type RunResult = {
  durationMs: number;
};

type BenchmarkResult = {
  id: string;
  label: string;
  debugRuns: number[]; // raw timings (ms) for is_debug=true
  cleanRuns: number[]; // raw timings (ms) for is_debug=false
  debugMedian: number;
  cleanMedian: number;
  overheadPct: number; // (debugMedian / cleanMedian - 1) * 100
};

type BenchmarkState =
  | { phase: 'idle' }
  | { phase: 'running'; current: string; mode: 'debug' | 'clean'; run: number }
  | { phase: 'done'; results: BenchmarkResult[] };

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

const median = (values: number[]): number => {
  const sorted = [...values].sort((a, b) => a - b);
  const mid = Math.floor(sorted.length / 2);
  return sorted.length % 2 !== 0 ? sorted[mid] : (sorted[mid - 1] + sorted[mid]) / 2;
};

async function timeRun(code: string, debug: boolean): Promise<RunResult> {
  const rt = Runtime.create('c');
  (rt as unknown as { debug: boolean }).debug = debug;
  rt.fs = { 'main.c': code };

  // Drain stdout/stderr so the streams don't back-pressure
  rt.stdout.pipeTo(new WritableStream({ write: () => {} })).catch(() => {});
  rt.stderr.pipeTo(new WritableStream({ write: () => {} })).catch(() => {});

  const t0 = performance.now();
  await rt.run();
  const t1 = performance.now();

  return { durationMs: t1 - t0 };
}

// ---------------------------------------------------------------------------
// Bar chart (SVG, no external deps)
// ---------------------------------------------------------------------------

const CHART_COLORS = { debug: '#ef4444', clean: '#22c55e' };

function BarChart({ results }: { results: BenchmarkResult[] }) {
  const maxVal = Math.max(...results.flatMap((r) => [r.debugMedian, r.cleanMedian]));
  const W = 600;
  const H = 300;
  const PAD = { top: 20, right: 20, bottom: 60, left: 70 };
  const innerW = W - PAD.left - PAD.right;
  const innerH = H - PAD.top - PAD.bottom;

  const groupW = innerW / results.length;
  const barW = (groupW - 16) / 2;
  const yScale = (v: number) => innerH - (v / maxVal) * innerH;

  return (
    <svg viewBox={`0 0 ${W} ${H}`} style={{ width: '100%', maxWidth: W }}>
      {/* Y axis label */}
      <text
        transform={`translate(14,${PAD.top + innerH / 2}) rotate(-90)`}
        textAnchor="middle"
        fontSize={12}
        fill="#6b7280"
      >
        Time (ms)
      </text>

      {/* Y gridlines */}
      {[0, 0.25, 0.5, 0.75, 1].map((frac) => {
        const y = PAD.top + frac * innerH;
        const val = maxVal * (1 - frac);
        return (
          <g key={frac}>
            <line
              x1={PAD.left}
              x2={PAD.left + innerW}
              y1={y}
              y2={y}
              stroke="#e5e7eb"
              strokeWidth={1}
            />
            <text x={PAD.left - 6} y={y + 4} textAnchor="end" fontSize={10} fill="#6b7280">
              {val.toFixed(0)}
            </text>
          </g>
        );
      })}

      {results.map((r, i) => {
        const gx = PAD.left + i * groupW + 8;
        const debugH = (r.debugMedian / maxVal) * innerH;
        const cleanH = (r.cleanMedian / maxVal) * innerH;

        return (
          <g key={r.id}>
            {/* debug bar */}
            <rect
              x={gx}
              y={PAD.top + yScale(r.debugMedian)}
              width={barW}
              height={debugH}
              fill={CHART_COLORS.debug}
              opacity={0.85}
            />
            {/* clean bar */}
            <rect
              x={gx + barW + 4}
              y={PAD.top + yScale(r.cleanMedian)}
              width={barW}
              height={cleanH}
              fill={CHART_COLORS.clean}
              opacity={0.85}
            />
            {/* x label */}
            <text
              x={gx + barW + 2}
              y={PAD.top + innerH + 16}
              textAnchor="middle"
              fontSize={10}
              fill="#374151"
            >
              {r.label.split(' ').map((word, wi) => (
                <tspan key={wi} x={gx + barW + 2} dy={wi === 0 ? 0 : 12}>
                  {word}
                </tspan>
              ))}
            </text>
          </g>
        );
      })}

      {/* Legend */}
      <rect x={PAD.left} y={H - 16} width={12} height={12} fill={CHART_COLORS.debug} />
      <text x={PAD.left + 16} y={H - 6} fontSize={11} fill="#374151">
        Instrumented (debug=true)
      </text>
      <rect x={PAD.left + 160} y={H - 16} width={12} height={12} fill={CHART_COLORS.clean} />
      <text x={PAD.left + 176} y={H - 6} fontSize={11} fill="#374151">
        Baseline (debug=false)
      </text>
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Overhead ratio chart
// ---------------------------------------------------------------------------

function OverheadChart({ results }: { results: BenchmarkResult[] }) {
  const maxVal = Math.max(...results.map((r) => r.overheadPct));
  const W = 600;
  const H = 240;
  const PAD = { top: 20, right: 20, bottom: 60, left: 70 };
  const innerW = W - PAD.left - PAD.right;
  const innerH = H - PAD.top - PAD.bottom;

  const barW = innerW / results.length - 16;

  return (
    <svg viewBox={`0 0 ${W} ${H}`} style={{ width: '100%', maxWidth: W }}>
      <text
        transform={`translate(14,${PAD.top + innerH / 2}) rotate(-90)`}
        textAnchor="middle"
        fontSize={12}
        fill="#6b7280"
      >
        Overhead (%)
      </text>

      {[0, 0.25, 0.5, 0.75, 1].map((frac) => {
        const y = PAD.top + frac * innerH;
        const val = maxVal * (1 - frac);
        return (
          <g key={frac}>
            <line
              x1={PAD.left}
              x2={PAD.left + innerW}
              y1={y}
              y2={y}
              stroke="#e5e7eb"
              strokeWidth={1}
            />
            <text x={PAD.left - 6} y={y + 4} textAnchor="end" fontSize={10} fill="#6b7280">
              {val.toFixed(0)}%
            </text>
          </g>
        );
      })}

      {results.map((r, i) => {
        const gx = PAD.left + i * (innerW / results.length) + 8;
        const barH = (r.overheadPct / maxVal) * innerH;

        return (
          <g key={r.id}>
            <rect
              x={gx}
              y={PAD.top + innerH - barH}
              width={barW}
              height={barH}
              fill="#6366f1"
              opacity={0.85}
            />
            <text
              x={gx + barW / 2}
              y={PAD.top + innerH - barH - 4}
              textAnchor="middle"
              fontSize={10}
              fill="#374151"
            >
              {r.overheadPct.toFixed(1)}%
            </text>
            <text
              x={gx + barW / 2}
              y={PAD.top + innerH + 16}
              textAnchor="middle"
              fontSize={10}
              fill="#374151"
            >
              {r.label.split(' ').map((word, wi) => (
                <tspan key={wi} x={gx + barW / 2} dy={wi === 0 ? 0 : 12}>
                  {word}
                </tspan>
              ))}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

// ---------------------------------------------------------------------------
// Results table
// ---------------------------------------------------------------------------

function ResultsTable({ results }: { results: BenchmarkResult[] }) {
  return (
    <table style={{ borderCollapse: 'collapse', width: '100%', fontSize: 13 }}>
      <thead>
        <tr style={{ borderBottom: '2px solid #e5e7eb' }}>
          {['Benchmark', 'Debug median (ms)', 'Clean median (ms)', 'Overhead'].map((h) => (
            <th key={h} style={{ padding: '8px 12px', textAlign: 'left', color: '#6b7280' }}>
              {h}
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        {results.map((r) => (
          <tr key={r.id} style={{ borderBottom: '1px solid #f3f4f6' }}>
            <td style={{ padding: '8px 12px', fontWeight: 500 }}>{r.label}</td>
            <td style={{ padding: '8px 12px', color: '#ef4444' }}>{r.debugMedian.toFixed(0)}</td>
            <td style={{ padding: '8px 12px', color: '#22c55e' }}>{r.cleanMedian.toFixed(0)}</td>
            <td style={{ padding: '8px 12px', color: '#6366f1', fontWeight: 600 }}>
              +{r.overheadPct.toFixed(1)}%
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

// ---------------------------------------------------------------------------
// Main page
// ---------------------------------------------------------------------------

const RUNS_PER_BENCHMARK = 5;

export default function BenchmarkPage() {
  const [state, setState] = useState<BenchmarkState>({ phase: 'idle' });
  const cancelRef = useRef(false);

  const runBenchmarks = useCallback(async (subset = BENCHMARKS) => {
    cancelRef.current = false;
    const results: BenchmarkResult[] = [];

    for (const bench of subset) {
      if (cancelRef.current) break;

      const debugRuns: number[] = [];
      const cleanRuns: number[] = [];

      for (const mode of ['debug', 'clean'] as const) {
        for (let run = 1; run <= RUNS_PER_BENCHMARK; run++) {
          if (cancelRef.current) break;

          setState({
            phase: 'running',
            current: bench.label,
            mode,
            run,
          });

          const { durationMs } = await timeRun(bench.code, mode === 'debug');
          if (mode === 'debug') debugRuns.push(durationMs);
          else cleanRuns.push(durationMs);
        }
      }

      if (debugRuns.length && cleanRuns.length) {
        const dm = median(debugRuns);
        const cm = median(cleanRuns);
        results.push({
          id: bench.id,
          label: bench.label,
          debugRuns,
          cleanRuns,
          debugMedian: dm,
          cleanMedian: cm,
          overheadPct: (dm / cm - 1) * 100,
        });
      }
    }

    if (!cancelRef.current) setState({ phase: 'done', results });
  }, []);

  const exportJSON = useCallback(() => {
    if (state.phase !== 'done') return;
    const blob = new Blob([JSON.stringify(state.results, null, 2)], {
      type: 'application/json',
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'benchmark_results.json';
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
  }, [state]);

  return (
    <div
      style={{ maxWidth: 720, margin: '0 auto', padding: '32px 16px', fontFamily: 'sans-serif' }}
    >
      <h1 style={{ fontSize: 24, fontWeight: 700, marginBottom: 4 }}>
        Instrumentation Overhead Benchmark
      </h1>
      <p style={{ color: '#6b7280', marginBottom: 24, fontSize: 14 }}>
        Runs each program {RUNS_PER_BENCHMARK}× with <code>debug=true</code> (DWARF + WASM
        instrumentation) and <code>debug=false</code> (baseline). Median wall-clock time includes
        compile + link + execute.
      </p>

      {/* Benchmark list */}
      <div style={{ marginBottom: 24 }}>
        {BENCHMARKS.map((b) => (
          <div
            key={b.id}
            style={{
              padding: '8px 12px',
              marginBottom: 8,
              background: '#f9fafb',
              borderRadius: 6,
              border: '1px solid #e5e7eb',
              display: 'flex',
              alignItems: 'center',
              gap: 12,
            }}
          >
            <button
              onClick={() => runBenchmarks([b])}
              disabled={state.phase === 'running'}
              style={{
                padding: '4px 12px',
                background: state.phase === 'running' ? '#e5e7eb' : '#4f46e5',
                color: state.phase === 'running' ? '#9ca3af' : '#fff',
                border: 'none',
                borderRadius: 4,
                cursor: state.phase === 'running' ? 'not-allowed' : 'pointer',
                fontWeight: 600,
                fontSize: 12,
                flexShrink: 0,
              }}
            >
              Run
            </button>
            <span style={{ fontWeight: 600 }}>{b.label}</span>
            <span style={{ color: '#6b7280', fontSize: 13 }}>{b.description}</span>
          </div>
        ))}
      </div>

      {/* Controls */}
      <div style={{ display: 'flex', gap: 12, marginBottom: 32 }}>
        <button
          onClick={runBenchmarks}
          disabled={state.phase === 'running'}
          style={{
            padding: '10px 24px',
            background: state.phase === 'running' ? '#9ca3af' : '#4f46e5',
            color: '#fff',
            border: 'none',
            borderRadius: 6,
            cursor: state.phase === 'running' ? 'not-allowed' : 'pointer',
            fontWeight: 600,
            fontSize: 14,
          }}
        >
          {state.phase === 'running' ? 'Running…' : 'Run Benchmarks'}
        </button>

        {state.phase === 'done' && (
          <button
            onClick={exportJSON}
            style={{
              padding: '10px 24px',
              background: '#fff',
              color: '#374151',
              border: '1px solid #d1d5db',
              borderRadius: 6,
              cursor: 'pointer',
              fontWeight: 600,
              fontSize: 14,
            }}
          >
            Export JSON
          </button>
        )}
      </div>

      {/* Progress */}
      {state.phase === 'running' && (
        <div
          style={{
            padding: '12px 16px',
            background: '#eff6ff',
            border: '1px solid #bfdbfe',
            borderRadius: 6,
            marginBottom: 24,
            fontSize: 14,
          }}
        >
          <strong>{state.current}</strong> — {state.mode === 'debug' ? 'instrumented' : 'baseline'}{' '}
          run {state.run}/{RUNS_PER_BENCHMARK}
        </div>
      )}

      {/* Results */}
      {state.phase === 'done' && (
        <>
          <h2 style={{ fontSize: 18, fontWeight: 600, marginBottom: 16 }}>Results</h2>
          <ResultsTable results={state.results} />

          <h2 style={{ fontSize: 18, fontWeight: 600, margin: '32px 0 16px' }}>
            Execution Time (ms)
          </h2>
          <BarChart results={state.results} />

          <h2 style={{ fontSize: 18, fontWeight: 600, margin: '32px 0 16px' }}>
            Overhead vs Baseline
          </h2>
          <OverheadChart results={state.results} />

          <p style={{ marginTop: 24, fontSize: 12, color: '#9ca3af' }}>
            Times are median of {RUNS_PER_BENCHMARK} runs. Wall-clock time includes Clang
            compilation, wasm-ld linking, DWARF parsing, WASM instrumentation (debug=true only), and
            execution.
          </p>
        </>
      )}
    </div>
  );
}

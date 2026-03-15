"""
Generate paper-quality benchmark figures from benchmark_results.json.

Usage:
    python benchmark_plot.py                       # reads benchmark_results.json in cwd
    python benchmark_plot.py path/to/results.json

Outputs (in the same directory as the input file):
    fig_times.pdf       — grouped bar chart: instrumented vs baseline (log scale)
    fig_slowdown.pdf    — slowdown factor (×N) per benchmark
"""

import json
import sys
import os
import statistics
import matplotlib
import matplotlib.pyplot as plt
import matplotlib.ticker as ticker
import numpy as np

matplotlib.rcParams.update({
    'font.family': 'serif',
    'font.size': 10,
    'axes.titlesize': 11,
    'axes.labelsize': 10,
    'legend.fontsize': 9,
    'xtick.labelsize': 9,
    'ytick.labelsize': 9,
    'figure.dpi': 150,
    'pdf.fonttype': 42,
    'ps.fonttype': 42,
    'axes.spines.top': False,
    'axes.spines.right': False,
})

# ---------------------------------------------------------------------------
# Load data
# ---------------------------------------------------------------------------

path = sys.argv[1] if len(sys.argv) > 1 else 'benchmark_results.json'
out_dir = os.path.dirname(os.path.abspath(path))

with open(path) as f:
    data = json.load(f)

# Short labels — full names go in the figure caption
short_labels = {
    'fib':    'Fibonacci\n(recursive)',
    'sort':   'Bubble\nSort',
    'sieve':  'Sieve of\nEratosthenes',
    'matmul': 'Matrix\nMultiply',
}
xlabels     = [short_labels.get(r['id'], r['label']) for r in data]
debug_med   = [r['debugMedian'] for r in data]
clean_med   = [r['cleanMedian'] for r in data]
slowdown    = [d / c for d, c in zip(debug_med, clean_med)]

def iqr_err(runs):
    s = sorted(runs)
    mid = len(s) // 2
    q1  = statistics.median(s[:mid])
    q3  = statistics.median(s[mid + len(s) % 2:])
    med = statistics.median(s)
    return med - q1, q3 - med   # lower, upper

debug_err = [iqr_err(r['debugRuns'])  for r in data]
clean_err = [iqr_err(r['cleanRuns']) for r in data]

n = len(data)
x = np.arange(n)

COLOR_DEBUG = '#c0392b'   # muted red
COLOR_CLEAN = '#27ae60'   # muted green
COLOR_SLOW  = '#2980b9'   # blue for slowdown

# ---------------------------------------------------------------------------
# Figure 1 — grouped bar chart, log y-axis
# ---------------------------------------------------------------------------

fig, ax = plt.subplots(figsize=(5.8, 3.4))

w = 0.32
bars_d = ax.bar(x - w/2, debug_med, w,
                label='Instrumented', color=COLOR_DEBUG, zorder=3,
                yerr=np.array(debug_err).T, capsize=3,
                error_kw=dict(linewidth=0.8, zorder=4))
bars_c = ax.bar(x + w/2, clean_med, w,
                label='Baseline', color=COLOR_CLEAN, zorder=3,
                yerr=np.array(clean_err).T, capsize=3,
                error_kw=dict(linewidth=0.8, zorder=4))

ax.set_yscale('log')
ax.set_ylabel('Wall-clock time (ms, log scale)')
ax.set_xticks(x)
ax.set_xticklabels(xlabels, ha='center', linespacing=1.3)
ax.yaxis.grid(True, which='both', linestyle='--', linewidth=0.4, alpha=0.7, zorder=0)
ax.set_axisbelow(True)

# Clean up y-axis tick labels: show plain numbers, not 10^x notation
ax.yaxis.set_major_formatter(ticker.FuncFormatter(
    lambda v, _: f'{int(v):,}' if v >= 1 else f'{v:.1f}'
))
ax.yaxis.set_minor_formatter(ticker.NullFormatter())

ax.legend(frameon=False, loc='upper right')
ax.set_xlim(-0.6, n - 0.4)

fig.tight_layout(pad=1.2)
out1 = os.path.join(out_dir, 'fig_times.pdf')
fig.savefig(out1, bbox_inches='tight')
print(f'Saved {out1}')

# ---------------------------------------------------------------------------
# Figure 2 — slowdown factor (×N)
# ---------------------------------------------------------------------------

fig, ax = plt.subplots(figsize=(5.8, 3.2))

bars = ax.bar(x, slowdown, color=COLOR_SLOW, zorder=3, width=0.5)

# Annotate each bar with the ×N label
for bar, s in zip(bars, slowdown):
    label = f'{s:.1f}×' if s < 100 else f'{s:.0f}×'
    ax.text(bar.get_x() + bar.get_width() / 2,
            bar.get_height() * 1.06,
            label, ha='center', va='bottom', fontsize=9, fontweight='bold')

# Reference line at 1× (no overhead)
ax.axhline(1, color='#555', linewidth=0.8, linestyle='--', zorder=2)
ax.text(n - 0.55, 1.15, 'no overhead', fontsize=7.5, color='#555', va='bottom')

ax.set_yscale('log')
ax.set_ylabel('Slowdown factor (×, log scale)')
ax.set_xticks(x)
ax.set_xticklabels(xlabels, ha='center', linespacing=1.3)
ax.yaxis.grid(True, which='both', linestyle='--', linewidth=0.4, alpha=0.7, zorder=0)
ax.set_axisbelow(True)
ax.yaxis.set_major_formatter(ticker.FuncFormatter(lambda v, _: f'{v:g}×'))
ax.yaxis.set_minor_formatter(ticker.NullFormatter())

# Add a bit of headroom for the labels
ax.set_ylim(bottom=0.8, top=max(slowdown) * 3)
ax.set_xlim(-0.5, n - 0.5)

fig.tight_layout(pad=1.2)
out2 = os.path.join(out_dir, 'fig_slowdown.pdf')
fig.savefig(out2, bbox_inches='tight')
print(f'Saved {out2}')

# ---------------------------------------------------------------------------
# Print summary table
# ---------------------------------------------------------------------------

print()
print(f"{'Benchmark':<26} {'Instrumented (ms)':>18} {'Baseline (ms)':>14} {'Slowdown':>10}")
print('-' * 72)
for r, s in zip(data, slowdown):
    print(f"{r['label']:<26} {r['debugMedian']:>18.0f} {r['cleanMedian']:>14.0f} {s:>9.1f}×")

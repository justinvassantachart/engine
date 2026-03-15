'use client';

import ExpandMoreIcon from '@mui/icons-material/ExpandMore';
import Box from '@mui/material/Box';
import Collapse from '@mui/material/Collapse';
import IconButton from '@mui/material/IconButton';
import Typography from '@mui/material/Typography';
import React, { useState } from 'react';
import type { BreakpointHit } from 'runtime';

type VariableLike = { name: string; ty: string; value: string };

type VariablesPanelProps = {
  /** When null, panel shows empty state. */
  hit: BreakpointHit | null;
};

export default function VariablesPanel({ hit }: VariablesPanelProps) {
  const [expandedFrame, setExpandedFrame] = useState<number>(0);

  if (hit == null) {
    return (
      <Box
        sx={{
          p: 2,
          height: '100%',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'rgba(255, 255, 255, 0.5)',
          fontSize: '0.813rem',
          textAlign: 'center',
          gap: 1,
        }}
      >
        <span>Run code and stop at a breakpoint to inspect variables and call stack.</span>
        <span style={{ fontSize: '0.75rem', opacity: 0.85 }}>
          Click in the gutter (left of line numbers) to set a breakpoint.
        </span>
      </Box>
    );
  }

  const { location, frames } = hit;
  const frameList = [...frames];

  return (
    <Box sx={{ height: '100%', overflow: 'auto' }}>
      <Box sx={{ px: 1.5, py: 1, borderBottom: '1px solid rgba(148, 163, 184, 0.2)' }}>
        <Typography
          variant="caption"
          sx={{
            color: 'rgba(255,255,255,0.6)',
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
          }}
        >
          Paused at {location.file}:{location.line}
        </Typography>
      </Box>
      <Typography
        variant="subtitle2"
        sx={{ px: 1.5, py: 0.75, color: 'rgba(255,255,255,0.7)', fontSize: '0.75rem' }}
      >
        Call stack
      </Typography>
      {frameList.map((frame, idx) => {
        const isExpanded = expandedFrame === idx;
        const vars = frame.variables();
        return (
          <Box key={idx} sx={{ borderBottom: '1px solid rgba(148, 163, 184, 0.1)' }}>
            <Box
              sx={{
                display: 'flex',
                alignItems: 'center',
                gap: 0.5,
                px: 1.5,
                py: 0.5,
                cursor: 'pointer',
                '&:hover': { bgcolor: 'rgba(255,255,255,0.05)' },
              }}
              onClick={() => setExpandedFrame(isExpanded ? -1 : idx)}
            >
              <IconButton
                size="small"
                sx={{ p: 0.25 }}
                aria-label={isExpanded ? 'Collapse' : 'Expand'}
              >
                <ExpandMoreIcon
                  sx={{
                    fontSize: '1rem',
                    transform: isExpanded ? 'rotate(180deg)' : 'rotate(0deg)',
                    color: 'rgba(255,255,255,0.7)',
                  }}
                />
              </IconButton>
              <Typography component="span" sx={{ fontFamily: 'monospace', fontSize: '0.813rem' }}>
                {frame.name || `function_${frame.functionIndex}`}
              </Typography>
            </Box>
            <Collapse in={isExpanded}>
              <Box sx={{ pl: 3, pr: 1.5, pb: 1 }}>
                {vars.length === 0 ? (
                  <Typography variant="caption" sx={{ color: 'rgba(255,255,255,0.45)' }}>
                    No variables
                  </Typography>
                ) : (
                  <Box
                    component="ul"
                    sx={{ m: 0, pl: 2, fontSize: '0.813rem', fontFamily: 'monospace' }}
                  >
                    {(vars as VariableLike[]).map((v, i) => (
                      <Box
                        component="li"
                        key={i}
                        sx={{ mb: 0.25, color: 'rgba(255,255,255,0.85)' }}
                      >
                        <Box component="span" sx={{ color: 'rgba(167, 139, 250, 0.95)' }}>
                          {v.ty}
                        </Box>{' '}
                        <Box component="span" sx={{ color: '#7dd3fc' }}>
                          {v.name}
                        </Box>
                        {' = '}
                        <Box component="span" sx={{ color: '#86efac' }}>
                          {v.value}
                        </Box>
                      </Box>
                    ))}
                  </Box>
                )}
              </Box>
            </Collapse>
          </Box>
        );
      })}
    </Box>
  );
}

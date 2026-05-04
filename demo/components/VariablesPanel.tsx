'use client';

import ChevronRightIcon from '@mui/icons-material/ChevronRight';
import ExpandMoreIcon from '@mui/icons-material/ExpandMore';
import { Box, CircularProgress, Collapse, Typography } from '@mui/material';
import React, { useCallback, useState } from 'react';

export type StackFrameRow = {
  id: number;
  name: string;
  line?: number;
  path?: string;
};

export type VarNode = {
  name: string;
  value: string;
  type?: string;
  variablesReference: number;
};

export type ScopeBlock = {
  name: string;
  variables: VarNode[];
};

type ExpandableVarProps = {
  node: VarNode;
  depth: number;
  childMap: Readonly<Record<number, VarNode[]>>;
  onExpand: (variablesReference: number) => void | Promise<void>;
};

function ExpandableVar({ node, depth, childMap, onExpand }: ExpandableVarProps) {
  const [open, setOpen] = useState(false);
  const expandable = node.variablesReference > 0;
  const children = childMap[node.variablesReference];

  const toggle = useCallback(() => {
    if (!expandable) return;
    if (!open && children === undefined) {
      void onExpand(node.variablesReference);
    }
    setOpen((o) => !o);
  }, [children, expandable, node.variablesReference, onExpand, open]);

  return (
    <Box sx={{ pl: depth * 1.25 }}>
      <Box
        onClick={toggle}
        sx={{
          display: 'flex',
          alignItems: 'flex-start',
          gap: 0.25,
          py: 0.2,
          cursor: expandable ? 'pointer' : 'default',
          fontSize: '0.72rem',
          fontFamily:
            'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace',
          '&:hover': expandable ? { backgroundColor: 'rgba(255,255,255,0.04)' } : undefined,
          borderRadius: 0.5
        }}
      >
        <Box
          sx={{ width: 18, flexShrink: 0, display: 'flex', justifyContent: 'center', mt: '-1px' }}
        >
          {expandable ? (
            open ? (
              <ExpandMoreIcon sx={{ fontSize: 16, color: 'rgba(255,255,255,0.45)' }} />
            ) : (
              <ChevronRightIcon sx={{ fontSize: 16, color: 'rgba(255,255,255,0.45)' }} />
            )
          ) : (
            <Box sx={{ width: 16 }} />
          )}
        </Box>
        <Box sx={{ minWidth: 0, flex: 1 }}>
          <Typography component="span" sx={{ color: '#93c5fd', fontSize: 'inherit' }}>
            {node.name}
          </Typography>
          {node.type ? (
            <Typography
              component="span"
              sx={{ color: 'rgba(255,255,255,0.35)', fontSize: '0.65rem', ml: 0.5 }}
            >
              {node.type}
            </Typography>
          ) : null}
          <Typography
            component="div"
            sx={{
              color: 'rgba(226, 232, 240, 0.85)',
              fontSize: '0.68rem',
              wordBreak: 'break-word',
              whiteSpace: 'pre-wrap'
            }}
          >
            {node.value}
          </Typography>
        </Box>
      </Box>
      {expandable ? (
        <Collapse in={open} timeout="auto" unmountOnExit>
          {children && children.length > 0 ? (
            children.map((ch, i) => (
              <ExpandableVar
                key={`${node.name}-${i}-${ch.name}`}
                node={ch}
                depth={depth + 1}
                childMap={childMap}
                onExpand={onExpand}
              />
            ))
          ) : open && children === undefined ? (
            <Box sx={{ pl: 3, py: 0.5 }}>
              <CircularProgress size={12} sx={{ color: 'rgba(165, 180, 252, 0.6)' }} />
            </Box>
          ) : open && Array.isArray(children) && children.length === 0 ? (
            <Typography
              sx={{ pl: 3, py: 0.25, fontSize: '0.65rem', color: 'rgba(255,255,255,0.35)' }}
            >
              (empty)
            </Typography>
          ) : null}
        </Collapse>
      ) : null}
    </Box>
  );
}

type VariablesPanelProps = {
  isPaused: boolean;
  debugLoading: boolean;
  frames: StackFrameRow[];
  selectedFrameId: number;
  onSelectFrame: (frameId: number) => void;
  scopes: ScopeBlock[];
  varChildMap: Readonly<Record<number, VarNode[]>>;
  onExpandVariable: (variablesReference: number) => void | Promise<void>;
};

export default function VariablesPanel({
  isPaused,
  debugLoading,
  frames,
  selectedFrameId,
  onSelectFrame,
  scopes,
  varChildMap,
  onExpandVariable
}: VariablesPanelProps) {
  const empty = !isPaused && frames.length === 0;

  return (
    <Box
      sx={{
        width: { xs: 0, sm: 260, md: 300 },
        display: { xs: 'none', sm: 'flex' },
        flexDirection: 'column',
        flexShrink: 0,
        borderLeft: '1px solid rgba(148, 163, 184, 0.12)',
        background: 'rgba(6, 8, 12, 0.55)',
        overflow: 'hidden',
        minHeight: 0
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
          borderBottom: '1px solid rgba(148, 163, 184, 0.1)'
        }}
      >
        Call stack
      </Typography>
      <Box sx={{ maxHeight: '28%', overflowY: 'auto', flexShrink: 0 }}>
        {empty ? (
          <Typography
            sx={{
              px: 1.25,
              py: 1,
              fontSize: '0.72rem',
              color: 'rgba(255,255,255,0.38)',
              lineHeight: 1.4
            }}
          >
            Run the program. When a breakpoint hits, frames appear here.
          </Typography>
        ) : debugLoading && frames.length === 0 ? (
          <Box sx={{ display: 'flex', justifyContent: 'center', py: 2 }}>
            <CircularProgress size={18} sx={{ color: 'rgba(165, 180, 252, 0.7)' }} />
          </Box>
        ) : (
          frames.map((f) => (
            <Box
              key={f.id}
              onClick={() => onSelectFrame(f.id)}
              sx={{
                px: 1.1,
                py: 0.45,
                cursor: 'pointer',
                fontSize: '0.72rem',
                fontFamily:
                  'ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, "Liberation Mono", "Courier New", monospace',
                borderLeft:
                  selectedFrameId === f.id ? '2px solid #818cf8' : '2px solid transparent',
                backgroundColor:
                  selectedFrameId === f.id ? 'rgba(99, 102, 241, 0.12)' : 'transparent',
                '&:hover': { backgroundColor: 'rgba(255,255,255,0.04)' }
              }}
            >
              <Typography
                sx={{
                  fontSize: 'inherit',
                  color: '#e2e8f0',
                  fontWeight: selectedFrameId === f.id ? 600 : 400
                }}
              >
                {f.name}
                {f.line != null ? (
                  <Typography
                    component="span"
                    sx={{ color: 'rgba(255,255,255,0.45)', fontWeight: 400, ml: 0.5 }}
                  >
                    :{f.line}
                  </Typography>
                ) : null}
              </Typography>
              {f.path ? (
                <Typography
                  sx={{
                    fontSize: '0.62rem',
                    color: 'rgba(255,255,255,0.35)',
                    wordBreak: 'break-all'
                  }}
                >
                  {f.path}
                </Typography>
              ) : null}
            </Box>
          ))
        )}
      </Box>

      <Typography
        sx={{
          px: 1.25,
          py: 0.65,
          fontSize: '0.62rem',
          fontWeight: 600,
          letterSpacing: '0.06em',
          textTransform: 'uppercase',
          color: 'rgba(255,255,255,0.45)',
          borderTop: '1px solid rgba(148, 163, 184, 0.1)',
          borderBottom: '1px solid rgba(148, 163, 184, 0.1)'
        }}
      >
        Variables
      </Typography>
      <Box sx={{ flex: 1, minHeight: 0, overflowY: 'auto', py: 0.5 }}>
        {!isPaused && scopes.length === 0 ? (
          <Typography
            sx={{ px: 1.25, py: 0.75, fontSize: '0.72rem', color: 'rgba(255,255,255,0.38)' }}
          >
            Locals and arguments show here while paused.
          </Typography>
        ) : debugLoading && scopes.length === 0 ? (
          <Box sx={{ display: 'flex', justifyContent: 'center', py: 2 }}>
            <CircularProgress size={18} sx={{ color: 'rgba(165, 180, 252, 0.7)' }} />
          </Box>
        ) : (
          scopes.map((scope) => (
            <Box key={scope.name} sx={{ mb: 1 }}>
              <Typography
                sx={{
                  px: 1.25,
                  py: 0.35,
                  fontSize: '0.65rem',
                  fontWeight: 600,
                  color: 'rgba(199, 210, 254, 0.9)',
                  letterSpacing: '0.04em'
                }}
              >
                {scope.name}
              </Typography>
              {scope.variables.length === 0 ? (
                <Typography sx={{ px: 1.5, fontSize: '0.68rem', color: 'rgba(255,255,255,0.32)' }}>
                  (none)
                </Typography>
              ) : (
                scope.variables.map((v, i) => (
                  <ExpandableVar
                    key={`${scope.name}-${i}-${v.name}`}
                    node={v}
                    depth={0}
                    childMap={varChildMap}
                    onExpand={onExpandVariable}
                  />
                ))
              )}
            </Box>
          ))
        )}
      </Box>
    </Box>
  );
}

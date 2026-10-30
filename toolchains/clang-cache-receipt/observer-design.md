# Observation-only implementation decisions

The exact unmodified SDK29 control must pass the committed structural controls,
complete artifact-difference audit and paired actual-browser gate before the
LLVM patch is applied or the observer build can run. This document is design,
not that acceptance. The receipt schema/ABI remain `receipt.schema.json` and
`webide_compiler.cache_receipt_v1(i32,i32)->i32`.

## Allocation and attachment reconciliation

The plan's early `DependencyCollector` subclass remains appropriate, but the
stock `CompilerInstance.addDependencyCollector(shared_ptr)` grows a vector and
allocates a shared_ptr control block. PP and AST listener chaining also use
throwing allocation. Optional observer setup must not turn memory pressure into
an ordinary compiler failure. Use a separate non-owning receipt collector slot,
registered by the supported `CodeGenAction::BeginInvocation` before preprocessor
creation, attached at the existing preprocessor and pre-ReadAST attachment
points. The action owns a nothrow-allocated observer. Callback installation must
use a nothrow chain operation that leaves existing callbacks untouched on
failure. All receipt storage and traversal are explicitly bounded; exhaustion
emits an unsupported receipt if possible, otherwise no receipt.

The observer never invokes stock dependency collectors or dependency-file
output, never returns true from missing-embed handling, and never changes the
normal source/PCH validation or diagnostic path. Builtin evidence is recorded
inside actual `Preprocessor::ExpandBuiltinMacro`, before its token mutation;
user-redefined names do not enter this path. The callback records only the three
volatile builtins and informational counter use.

## Actual-loaded PCH reconciliation

The original plan suggested `needsSystemInputFileVisitation=true` and
`visitInputFileInfos`. Actual unchanged-compiler controls prove this would be
unsafe: a malformed name length or timestamp/flag in an otherwise-unused system
record can preserve ordinary successful compilation and exact object bytes.
The pinned stock getInputFileInfo uses unchecked Expected values and asserts on
record/name structure. Enabling all-input visitation would force that unsafe
path and change ordinary behavior.

Attach before ReadAST to retain the actual ASTReader, but do not request stock
input visitation. Inspect the actual loaded ModuleFile's bytes and parsed state
without filesystem reads, AST deserialization or mutation. Require an exact
set of directly loaded MK_PCH files, no imported/chained/modules/remaps, the
parsed HasTimestamps flag false, and every bounded checked input record's
stored time zero and override/transient/module-map flags false. The reader
checks all user and system records. Retain narrow read-only metadata such as
input-offset blob length/block end if necessary to check exact bounds rather
than trusting pointers into adjacent records. Validate the pinned canonical
abbreviation schema before decoding with a no-allocation checked bit reader.
Unknown formats decline. No hashing equality or finite output comparison
substitutes for this parsed proof.

Seven isolated actual PCH mutation controls are preserved in the implementation
evidence directory and must continue to produce the ordinary outcome with the
observer on, while unsafe metadata never grants eligibility. The independent
Python bitstream reader remains test evidence, not runtime authority.

## Observation and completion

Collect main input, entered and successfully skipped headers, all exact loaded
PCH input records, and positive include-probe targets. Capture include probes'
active physical lexer file rather than macro spelling locations. All paths are
bounded and lexically canonical under the separately enforced root-cwd/no-alias
engine environment. No observer filesystem queries are added.

Active embed directives/builtins are unsupported. Inspect already source-parsed
AST nodes with a bounded traversal (no lazy external declarations), plus actual
emitted LLVM IR before backend passes for nonempty assembly restored from PCH.
Empty barriers and symbol-label attributes retain their ordinary meanings.
Completion occurs explicitly in the object frontend action, not a destructor;
-disable-free is preserved. Host return values are ignored, ordinary output and
exit remain authoritative, and the host validates the provisional bounded
record only after successful silent object compilation.

The option belongs only to FrontendOptions. It must never enter LangOptions,
PCH serialized compatibility, macro definitions, header-search settings or
code-generation options. Off/on PCH creation and consumption are paired with
exact-byte controls. The private import must be safely adapted for every
multi-tool module entrypoint including wasm-ld, even with observation disabled.

The pinned INPUT_FILE abbreviation is literal1, VBR6 id, VBR12 size, VBR32
mtime, four fixed1 flags, VBR16 requested-name length, and a blob; INPUT_FILE_HASH
is literal2 plus two fixed32 words. The actual input block uses a four-bit code
width. Verify these exact abbreviation descriptors without an allocating error
path, then read each record from the actual loaded Data buffer at its checked
input-offset-table entry. Require ordered exact ids, bounded blob/name lengths,
a following hash record and exact next-record/block boundaries. A malformed
unused system record must only change eligibility. Full source paths, metadata
flags and content closure remain keyed by the engine's owned-byte policy.

## Implemented bounded source review

The source-AST path uses explicit read-only FunctionDecl and VarDecl accessors
that return only already loaded bodies/initializers, never the normal getBody
redeclaration walk or a lazy initializer load. It skips imported declarations
and uses noload_decls. AST and IR traversal each cap one million visited nodes
or instructions; the AST pending stack caps 65,536 entries.

Receipt, callback and optional callback-chain classes use malloc-backed
class-specific nothrow allocation and matching delete; no global throwing-new
behavior is assumed. Failure leaves existing callbacks untouched and disables
eligibility. Both has_include and has_include_next use the same exact parsed
probe observation; the engine validates the conservative search-prefix superset.

## Native API validation correction

The first candidate failed the generated-header native syntax check before the
remote build completed. The exact failed patch, logs and source commit remain
in evidence. The pinned GCC/FileScope assembly API returns a newly allocated
std::string; the observer now reads the already-held StringLiteral view through
getAsmStringExpr. Nonliteral assembly expressions decline without evaluation or
allocation. Empty literal barriers and symbol-label attributes stay eligible.

The pre-PCH factory is static. Its optional raw receipt-collector parameter now
defaults to null and is passed explicitly by the instance-owned factory before
ReadAST. Existing static callers keep the null default and ordinary semantics.
All nine changed-file/integration native syntax checks pass; this does not
replace the actual SDK29 WASM build, import or semantic/browser gates.

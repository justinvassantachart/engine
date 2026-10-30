//! Fail-closed receipt consumer. No shipped toolchain currently grants this capability.
//! The parser and identity rules can be tested independently of an unvalidated compiler.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const CAPABILITY: &str = "webide-object-cache-v1";
pub const CACHE_SCHEMA: &str = "debugger-sh/cpp-tu/receipt-v1";
pub const ABI: &str = "wasm32-wasip1/c++23/legacy-eh/receipt-v1";
pub const MAX_RECEIPT_BYTES: usize = 1024 * 1024;
const MAX_PATH: usize = 4096;
const LINK_PREFIX: &[&str] = &["--export-dynamic", "-z", "stack-size=1048576", "-L/lib/wasm32-wasip1", "/lib/wasm32-wasip1/crt1.o"];
const LINK_SUFFIX: &[&str] = &["/lib/wasm32-wasip1/cxa_exception.cpp.o", "/lib/wasm32-wasip1/cxa_personality.cpp.o", "-lc++", "-lc++abi", "-lunwind", "/lib/wasm32-unknown-wasip1/libclang_rt.builtins.a", "-lc", "-o", "/main.wasm"];

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    schema: u32,
    capability: String,
    completed: bool,
    source: String,
    volatile_builtins: Vec<VolatileBuiltin>,
    uses_counter: bool,
    unsupported: Vec<String>,
    dependencies: Vec<String>,
    include_probes: Vec<IncludeProbe>,
    imported_pch_paths: Vec<String>,
    pch_metadata_independent: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum VolatileBuiltin {
    #[serde(rename = "__DATE__")]
    Date,
    #[serde(rename = "__TIME__")]
    Time,
    #[serde(rename = "__TIMESTAMP__")]
    Timestamp,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct IncludeProbe {
    name: String,
    angled: bool,
    includer: String,
    // `Option` alone permits an omitted field in serde; the versioned wire
    // schema requires an explicit null for a negative query.
    #[serde(deserialize_with = "required_nullable_path")]
    found: Option<String>,
}

fn required_nullable_path<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bypass {
    MissingReceipt,
    MalformedReceipt,
    ReceiptBudget,
    DuplicateReceipt,
    UnsupportedInvocation,
    UnsupportedPath,
    OutputCollision,
    TemporalExpansion,
    UnsupportedConstruct,
    DependencyClosure,
    ProbeDomain,
    PchMetadata,
    Diagnostics,
    CompilerFailure,
}
impl Bypass {
    pub fn reason(self) -> &'static str {
        match self {
            Self::MissingReceipt => "receipt-missing", Self::MalformedReceipt => "receipt-malformed",
            Self::ReceiptBudget => "receipt-budget", Self::DuplicateReceipt => "receipt-duplicate",
            Self::UnsupportedInvocation => "receipt-unsupported-invocation", Self::UnsupportedPath => "receipt-unsupported-path",
            Self::OutputCollision => "receipt-output-collision", Self::TemporalExpansion => "receipt-temporal-expansion",
            Self::UnsupportedConstruct => "receipt-unsupported-construct", Self::DependencyClosure => "receipt-dependency-closure",
            Self::ProbeDomain => "receipt-probe-domain", Self::PchMetadata => "receipt-pch-metadata",
            Self::Diagnostics => "compiler-diagnostics", Self::CompilerFailure => "compiler-failure",
        }
    }
}

type Checked<T> = Result<T, Bypass>;

fn bounded_text(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max && !value.chars().any(|c| c.is_control())
}
fn canonical(path: &str) -> bool {
    bounded_text(path, MAX_PATH) && path.starts_with('/') && !path.contains('\\') && !path[1..].split('/').any(|p| p.is_empty() || p == "." || p == "..")
}
fn unique<T: Ord>(values: &[T]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}
fn digest(bytes: &[u8]) -> String { format!("{:x}", Sha256::digest(bytes)) }
fn add(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

pub fn parse(bytes: &[u8], metadata_limit: usize) -> Checked<Receipt> {
    if bytes.len() > metadata_limit.min(MAX_RECEIPT_BYTES) { return Err(Bypass::ReceiptBudget); }
    let receipt: Receipt = serde_json::from_slice(bytes).map_err(|_| Bypass::MalformedReceipt)?;
    if receipt.schema != 1 || receipt.capability != CAPABILITY || !receipt.completed ||
        !canonical(&receipt.source) || receipt.volatile_builtins.len() > 3 || !unique(&receipt.volatile_builtins) ||
        receipt.unsupported.len() > 64 || !unique(&receipt.unsupported) || receipt.unsupported.iter().any(|s| !bounded_text(s, 128)) ||
        receipt.dependencies.len() > 16384 || !unique(&receipt.dependencies) || receipt.dependencies.iter().any(|s| !canonical(s)) ||
        receipt.imported_pch_paths.len() > 16 || !unique(&receipt.imported_pch_paths) || receipt.imported_pch_paths.iter().any(|s| !canonical(s)) ||
        receipt.include_probes.len() > 8192 || receipt.include_probes.iter().any(|p| !bounded_text(&p.name, MAX_PATH) || !canonical(&p.includer) || p.found.as_ref().is_some_and(|s| !canonical(s))) {
        return Err(Bypass::MalformedReceipt);
    }
    Ok(receipt)
}

/// One compiler invocation owns one capture. No memory view or callback survives it.
#[derive(Debug)]
pub struct Capture {
    metadata_limit: usize,
    attempted: bool,
    result: Checked<Vec<u8>>,
}
impl Capture {
    pub fn new(metadata_limit: usize) -> Self {
        Self { metadata_limit: metadata_limit.min(MAX_RECEIPT_BYTES), attempted: false, result: Err(Bypass::MissingReceipt) }
    }
    /// Host-import ABI: 0 means provisionally captured; any other value declines.
    /// The compiler must ignore the return value. Object success/diagnostics are checked later.
    pub fn receive(&mut self, memory: &[u8], pointer: i32, length: i32) -> i32 {
        if self.attempted { self.result = Err(Bypass::DuplicateReceipt); return 1; }
        let range = usize::try_from(pointer).ok().zip(usize::try_from(length).ok())
            .and_then(|(start, len)| start.checked_add(len).map(|end| (start, end, len)));
        let (start, end, len) = match range {
            Some((_, _, len)) if self.metadata_limit == 0 || len > self.metadata_limit => return self.decline(Bypass::ReceiptBudget),
            Some(range) if range.1 <= memory.len() => range,
            _ => return self.decline(Bypass::MalformedReceipt),
        };
        let mut bytes = Vec::new();
        if bytes.try_reserve_exact(len).is_err() { return self.decline(Bypass::ReceiptBudget); }
        bytes.extend_from_slice(&memory[start..end]);
        self.receive_owned(bytes)
    }
    /// The Wasmer adapter has already made its one bounded memory copy. Parsing
    /// and every admission decision wait until the compiler action has settled.
    pub fn receive_owned(&mut self, bytes: Vec<u8>) -> i32 {
        if self.attempted { return self.decline(Bypass::DuplicateReceipt); }
        if self.metadata_limit == 0 || bytes.len() > self.metadata_limit { return self.decline(Bypass::ReceiptBudget); }
        self.attempted = true;
        self.result = Ok(bytes);
        0
    }
    pub fn limit(&self) -> usize { self.metadata_limit }
    pub fn attempted(&self) -> bool { self.attempted }
    pub fn decline(&mut self, reason: Bypass) -> i32 {
        self.result = Err(if self.attempted { Bypass::DuplicateReceipt } else { reason });
        self.attempted = true;
        1
    }
    pub fn finish(self, exit_success: bool, diagnostic_bytes: u64) -> Checked<Receipt> {
        if !exit_success { return Err(Bypass::CompilerFailure); }
        if diagnostic_bytes != 0 { return Err(Bypass::Diagnostics); }
        parse(&self.result?, self.metadata_limit)
    }
}

/// This descriptor must be applied at the compiler WASI boundary before admission
/// can be enabled. It is internal policy, never a caller-supplied certification.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Environment {
    argv0: String,
    cwd: String,
    variables: BTreeMap<String, String>,
    preopens: Vec<String>,
    mount_order: Vec<String>,
    path_policy: String,
    resource_directory: String,
    builtin_module_map_directory: String,
    absent_global_module_index: String,
    header_search: Vec<String>,
    stdin_empty: bool,
}
impl Environment {
    pub fn fixed() -> Self {
        Self {
            argv0: "clang".into(), cwd: "/".into(), variables: BTreeMap::new(), preopens: vec!["/".into()],
            mount_order: vec!["verified-sysroot".into(), "verified-exceptions".into(), "owned-user-snapshot".into()],
            // Pinned build.sh uses -DCLANG_RESOURCE_DIR=/usr. Shipped cc1 -v
            // confirms these three search roots only; /usr/include is separately
            // probed for builtin module maps, not added as a header-search root.
            path_policy: "case-sensitive/canonical/no-symlinks/v1".into(), resource_directory: "/usr".into(),
            builtin_module_map_directory: "/usr/include".into(),
            // Normal PCH loading probes this even without module flags. Do not
            // change the compiler's global-index policy to manufacture closure.
            absent_global_module_index: "/modules.idx".into(),
            header_search: vec!["/include/c++/v1".into(), "/include".into(), "/include/wasm32-wasip1".into()], stdin_empty: true,
        }
    }
    fn supported(&self) -> bool { Self::fixed() == *self }
    pub fn arguments(&self, source: &str, pch: Option<&str>) -> Vec<String> {
        let mut args: Vec<String> = [
            "-cc1", "-triple", "wasm32-wasip1", "-Werror", "-emit-obj", "-disable-free", "-isysroot", "/",
            "-internal-isystem", "/include/c++/v1", "-internal-isystem", "/include", "-internal-isystem", "/include/wasm32-wasip1",
            "-ferror-limit", "4", "-fcolor-diagnostics", "-x", "c++", "-std=c++23", "-fexceptions", "-fcxx-exceptions", "-exception-model=wasm",
            "-target-feature", "+exception-handling", "-mllvm", "-wasm-enable-eh", "-mllvm", "-wasm-use-legacy-eh=true", "-o",
        ].into_iter().map(String::from).collect();
        args.push(format!("{source}.o"));
        if let Some(pch) = pch { args.extend(["-include-pch".into(), pch.into(), "-fvalidate-ast-input-files-content".into()]); }
        args.extend(["-resource-dir".into(), self.resource_directory.clone(), "-fwebide-cache-receipt".into(), source.into()]);
        args
    }

    pub fn link_arguments(sources: &[String], archives: &[String]) -> Vec<String> {
        LINK_PREFIX.iter().map(|s| s.to_string())
            .chain(sources.iter().map(|source| format!("{source}.o")))
            .chain(archives.iter().cloned())
            .chain(LINK_SUFFIX.iter().map(|s| s.to_string())).collect()
    }
}

#[derive(Debug, Clone)]
pub struct ToolchainIdentity {
    pub compiler: String,
    pub sysroot: String,
    pub exceptions: String,
    pub abi: String,
}
impl ToolchainIdentity {
    fn valid(&self) -> bool {
        [&self.compiler, &self.sysroot, &self.exceptions].iter().all(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))) && bounded_text(&self.abi, 128)
    }
    fn key(&self) -> String {
        let mut hash = Sha256::new();
        for field in [&self.compiler, &self.sysroot, &self.exceptions, &self.abi] { add(&mut hash, field.as_bytes()); }
        format!("{:x}", hash.finalize())
    }
}

/// Exact effective immutable namespace: toolchain first, then owned user overlays.
/// The toolchain member inventory must come from the verified archive, including
/// all accessible files. Fresh filesystem construction may not introduce extra inputs.
pub struct InputIndex {
    environment: Environment,
    toolchain: ToolchainIdentity,
    files: BTreeMap<String, String>,
    user_files: BTreeMap<String, String>,
    namespace: BTreeSet<String>,
    sources: Vec<String>,
    outputs: Vec<String>,
    pch: Option<String>,
    link_arguments: Vec<String>,
}

/// Hash the owned user snapshot once, before asynchronous filesystem construction.
pub struct UserInputs {
    files: BTreeMap<String, String>,
}
impl UserInputs {
    pub fn new<'a>(files: impl IntoIterator<Item = (String, &'a [u8])>) -> Checked<Self> {
        let mut result = BTreeMap::new();
        for (path, bytes) in files {
            if !canonical(&path) || result.contains_key(&path) { return Err(Bypass::UnsupportedPath); }
            result.insert(path, digest(bytes));
        }
        Ok(Self { files: result })
    }
}

impl InputIndex {
    pub fn new<'a>(environment: Environment, toolchain: ToolchainIdentity,
        toolchain_files: impl IntoIterator<Item = String>, user_files: impl IntoIterator<Item = (String, &'a [u8])>,
        mounted_directories: impl IntoIterator<Item = String>, sources: Vec<String>, pch: Option<String>, link_arguments: Vec<String>) -> Checked<Self> {
        Self::from_user_inputs(environment, toolchain, toolchain_files, UserInputs::new(user_files)?, mounted_directories, sources, pch, link_arguments)
    }

    pub fn from_user_inputs(environment: Environment, toolchain: ToolchainIdentity,
        toolchain_files: impl IntoIterator<Item = String>, user_files: UserInputs,
        mounted_directories: impl IntoIterator<Item = String>, sources: Vec<String>, pch: Option<String>, link_arguments: Vec<String>) -> Checked<Self> {
        if !environment.supported() || !toolchain.valid() || sources.is_empty() || !unique(&sources) || sources.len() > 256 ||
            sources.iter().any(|s| !canonical(s)) || pch.as_ref().is_some_and(|s| !canonical(s)) ||
            link_arguments.iter().any(|arg| arg.starts_with('@') || !bounded_text(arg, MAX_PATH)) {
            return Err(Bypass::UnsupportedInvocation);
        }
        let mut files = BTreeMap::new();
        let identity = toolchain.key();
        for path in toolchain_files {
            if !canonical(&path) || files.contains_key(&path) { return Err(Bypass::UnsupportedPath); }
            // All bytes of this member are bound by the actual verified archive identity.
            files.insert(path.clone(), digest(format!("{identity}:{path}").as_bytes()));
        }
        let user = user_files.files;
        files.extend(user.iter().map(|(path, content)| (path.clone(), content.clone())));
        if sources.iter().any(|s| !user.contains_key(s)) || pch.as_ref().is_some_and(|s| !user.contains_key(s)) { return Err(Bypass::DependencyClosure); }
        let archive_start = LINK_PREFIX.len() + sources.len();
        let archive_end = link_arguments.len().checked_sub(LINK_SUFFIX.len()).ok_or(Bypass::UnsupportedInvocation)?;
        if archive_end < archive_start { return Err(Bypass::UnsupportedInvocation); }
        let archives = &link_arguments[archive_start..archive_end];
        if archives.iter().any(|path| !canonical(path) || !files.contains_key(path)) ||
            link_arguments != Environment::link_arguments(&sources, archives) {
            return Err(Bypass::UnsupportedInvocation);
        }
        let mut namespace = BTreeSet::from(["/".to_string()]);
        let mut directories = BTreeSet::new();
        for path in mounted_directories {
            if path == "/" { continue; }
            if !canonical(&path) || files.contains_key(&path) { return Err(Bypass::UnsupportedPath); }
            directories.insert(path);
        }
        for path in files.keys().chain(directories.iter()) {
            namespace.insert(path.clone());
            let mut parent = path.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                if prefix.is_empty() { break; }
                if files.contains_key(prefix) { return Err(Bypass::UnsupportedPath); }
                namespace.insert(prefix.to_string()); parent = prefix;
            }
        }
        let mut outputs: Vec<_> = sources.iter().map(|source| format!("{source}.o")).collect();
        outputs.push("/main.wasm".into());
        let result = Self { environment, toolchain, files, user_files: user, namespace, sources, outputs, pch, link_arguments };
        if result.namespace.contains(&result.environment.absent_global_module_index) { return Err(Bypass::UnsupportedInvocation); }
        if result.namespace.iter().any(|path| result.generated(path)) { return Err(Bypass::OutputCollision); }
        Ok(result)
    }

    fn generated(&self, path: &str) -> bool {
        if ["/tmp", "/dev", "/proc"].iter().any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/"))) { return true; }
        self.outputs.iter().any(|output| {
            if path == output { return true; }
            let Some((stem, extension)) = output.rsplit_once('.') else { return true; };
            // Clang creates entropy-derived sibling temporaries before final rename.
            path.starts_with(&format!("{stem}-")) && path.ends_with(&format!(".{extension}.tmp"))
        })
    }

    pub fn base_key(&self, source: &str, arguments: &[String]) -> Checked<String> {
        if !self.sources.iter().any(|s| s == source) || !self.environment.supported() || arguments.iter().any(|arg| arg.starts_with('@')) || arguments != self.environment.arguments(source, self.pch.as_deref()) {
            return Err(Bypass::UnsupportedInvocation);
        }
        let mut hash = Sha256::new();
        for value in [CACHE_SCHEMA, CAPABILITY, self.toolchain.key().as_str(), source] { add(&mut hash, value.as_bytes()); }
        add(&mut hash, &serde_json::to_vec(&self.environment).map_err(|_| Bypass::UnsupportedInvocation)?);
        add(&mut hash, b"compile");
        for arg in arguments { add(&mut hash, arg.as_bytes()); }
        add(&mut hash, b"link");
        for arg in &self.link_arguments { add(&mut hash, arg.as_bytes()); }
        add(&mut hash, b"ordered-source-output");
        for (source, output) in self.sources.iter().zip(&self.outputs) { add(&mut hash, source.as_bytes()); add(&mut hash, output.as_bytes()); }
        add(&mut hash, b"namespace");
        for path in &self.namespace { add(&mut hash, path.as_bytes()); }
        add(&mut hash, b"broad-user-content");
        for (path, content) in &self.user_files {
            if path == source || !self.sources.contains(path) { add(&mut hash, path.as_bytes()); add(&mut hash, content.as_bytes()); }
        }
        Ok(format!("{:x}", hash.finalize()))
    }

    pub fn validate(&self, receipt: &Receipt, source: &str, arguments: &[String], exit_success: bool, diagnostic_bytes: u64) -> Checked<String> {
        if !exit_success { return Err(Bypass::CompilerFailure); }
        if diagnostic_bytes != 0 { return Err(Bypass::Diagnostics); }
        let base = self.base_key(source, arguments)?;
        if receipt.source != source || !receipt.dependencies.iter().any(|s| s == source) { return Err(Bypass::DependencyClosure); }
        if !receipt.volatile_builtins.is_empty() { return Err(Bypass::TemporalExpansion); }
        if !receipt.unsupported.is_empty() { return Err(Bypass::UnsupportedConstruct); }
        let expected_pch: Vec<_> = self.pch.iter().cloned().collect();
        if receipt.imported_pch_paths != expected_pch || !receipt.pch_metadata_independent || expected_pch.iter().any(|s| !receipt.dependencies.contains(s)) { return Err(Bypass::PchMetadata); }
        let mut dependencies = BTreeMap::new();
        for path in &receipt.dependencies {
            if self.generated(path) { return Err(Bypass::DependencyClosure); }
            dependencies.insert(path, self.files.get(path).ok_or(Bypass::DependencyClosure)?);
        }
        for probe in &receipt.include_probes {
            if !dependencies.contains_key(&probe.includer) { return Err(Bypass::ProbeDomain); }
            let candidates = self.candidates(probe)?;
            if candidates.iter().any(|path| self.generated(path)) { return Err(Bypass::ProbeDomain); }
            if let Some(found) = &probe.found {
                if !candidates.contains(found) || !dependencies.contains_key(found) { return Err(Bypass::ProbeDomain); }
            }
        }
        let mut hash = Sha256::new(); add(&mut hash, base.as_bytes());
        for (path, content) in dependencies { add(&mut hash, path.as_bytes()); add(&mut hash, content.as_bytes()); }
        Ok(format!("{:x}", hash.finalize()))
    }

    fn candidates(&self, probe: &IncludeProbe) -> Checked<BTreeSet<String>> {
        if probe.name.starts_with('/') { return Ok(BTreeSet::from([normalize("/", &probe.name)?])); }
        let parent = probe.includer.rsplit_once('/').map(|(p, _)| if p.is_empty() { "/" } else { p }).ok_or(Bypass::ProbeDomain)?;
        // Safe superset for quote/angle/include_next; no reconstruction of Clang's cursor.
        let mut roots = self.environment.header_search.clone(); roots.push(parent.into());
        roots.into_iter().map(|root| normalize(&root, &probe.name)).collect()
    }
}

fn normalize(root: &str, name: &str) -> Checked<String> {
    if !bounded_text(name, MAX_PATH) || name.contains('\\') { return Err(Bypass::ProbeDomain); }
    let combined = if name.starts_with('/') { name.into() } else { format!("{root}/{name}") };
    let mut parts = Vec::new();
    for part in combined.split('/') {
        match part {
            "" | "." => (),
            ".." => { if parts.pop().is_none() { return Err(Bypass::ProbeDomain); } },
            value => parts.push(value),
        }
    }
    let path = format!("/{}", parts.join("/"));
    if !canonical(&path) { return Err(Bypass::ProbeDomain); }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn wire() -> Value {
        json!({"schema":1,"capability":CAPABILITY,"completed":true,"source":"/a.cpp",
            "volatileBuiltins":[],"usesCounter":true,"unsupported":[],
            "dependencies":["/a.cpp","/include/stdio.h"],"includeProbes":[],
            "importedPchPaths":[],"pchMetadataIndependent":true})
    }
    fn read(value: Value) -> Checked<Receipt> { parse(&serde_json::to_vec(&value).unwrap(), MAX_RECEIPT_BYTES) }
    fn identity() -> ToolchainIdentity {
        ToolchainIdentity { compiler: "a".repeat(64), sysroot: "b".repeat(64), exceptions: "c".repeat(64), abi: "wasm32-wasip1/c++23/legacy-eh/receipt-v1".into() }
    }
    fn files() -> BTreeMap<String, Vec<u8>> {
        [("/a.cpp", "int a;"), ("/b.cpp", "int b;"), ("/common.h", "#define N 1"), ("/image.bin", "binary")]
            .into_iter().map(|(p, b)| (p.into(), b.as_bytes().to_vec())).collect()
    }
    fn index(files: &BTreeMap<String, Vec<u8>>, sources: &[&str], pch: Option<&str>) -> Checked<InputIndex> {
        let sources: Vec<_> = sources.iter().map(|s| s.to_string()).collect();
        InputIndex::new(Environment::fixed(), identity(),
            vec!["/include/stdio.h".into(), "/include/c++/v1/vector".into()],
            files.iter().map(|(p,b)| (p.clone(), b.as_slice())), vec![], sources.clone(), pch.map(String::from),
            Environment::link_arguments(&sources, &[]))
    }
    fn key(index: &InputIndex, receipt: &Receipt) -> Checked<String> {
        index.validate(receipt, "/a.cpp", &index.environment.arguments("/a.cpp", index.pch.as_deref()), true, 0)
    }

    #[test]
    fn strict_schema_requires_every_field_including_explicit_negative_probe() {
        let valid = wire();
        assert!(read(valid.clone()).is_ok());
        for field in valid.as_object().unwrap().keys() {
            let mut value = valid.clone(); value.as_object_mut().unwrap().remove(field);
            assert_eq!(read(value), Err(Bypass::MalformedReceipt), "missing {field}");
        }
        let mut value = valid;
        value["includeProbes"] = json!([{"name":"missing.h","angled":false,"includer":"/a.cpp","found":null}]);
        assert!(read(value.clone()).is_ok());
        value["includeProbes"][0].as_object_mut().unwrap().remove("found");
        assert_eq!(read(value), Err(Bypass::MalformedReceipt));
    }

    #[test]
    fn unknown_duplicate_wrong_type_and_invalid_utf8_are_rejected() {
        for (field, value) in [("schema", json!(2)), ("capability", json!("caller-authorized")), ("completed", json!(false)), ("usesCounter", json!(1)), ("extra", json!(null))] {
            let mut record = wire(); record[field] = value;
            assert_eq!(read(record), Err(Bypass::MalformedReceipt));
        }
        let mut value = wire(); value["includeProbes"] = json!([{"name":"x","angled":false,"includer":"/a.cpp","found":null,"extra":0}]);
        assert_eq!(read(value), Err(Bypass::MalformedReceipt));
        let mut bytes = serde_json::to_vec(&wire()).unwrap();
        bytes.pop(); bytes.extend_from_slice(b",\"schema\":1}");
        assert_eq!(parse(&bytes, MAX_RECEIPT_BYTES), Err(Bypass::MalformedReceipt));
        assert_eq!(parse(&[0xff], 1), Err(Bypass::MalformedReceipt));
    }

    #[test]
    fn string_array_and_total_budgets_are_enforced() {
        for path in ["/", "relative", "/a//b", "/a/../b", "/a/./b", "/a\\b", "/nul\0", "/a\n"] {
            let mut value = wire(); value["source"] = json!(path);
            assert_eq!(read(value), Err(Bypass::MalformedReceipt), "{path:?}");
        }
        let mut value = wire(); value["source"] = json!(format!("/{}", "a".repeat(MAX_PATH)));
        assert_eq!(read(value), Err(Bypass::MalformedReceipt));
        for (field, values) in [
            ("volatileBuiltins", json!(["__TIME__","__TIME__"])),
            ("unsupported", json!(["asm","asm"])),
            ("dependencies", json!(["/a.cpp","/a.cpp"])),
            ("importedPchPaths", json!(["/x.pch","/x.pch"])),
            ("unsupported", json!((0..65).map(|n| n.to_string()).collect::<Vec<_>>())),
            ("dependencies", json!((0..16385).map(|n| format!("/{n}")).collect::<Vec<_>>())),
            ("importedPchPaths", json!((0..17).map(|n| format!("/{n}")).collect::<Vec<_>>())),
            ("includeProbes", json!(vec![json!({"name":"x","angled":true,"includer":"/a.cpp","found":null});8193])),
        ] {
            let mut record = wire(); record[field] = values;
            assert_eq!(read(record), Err(Bypass::MalformedReceipt), "{field}");
        }
        let bytes = serde_json::to_vec(&wire()).unwrap();
        assert_eq!(parse(&bytes, bytes.len()-1), Err(Bypass::ReceiptBudget));
        assert!(parse(&bytes, bytes.len()).is_ok());
        assert_eq!(parse(&bytes, 0), Err(Bypass::ReceiptBudget));
        assert_eq!(parse(&vec![b' '; MAX_RECEIPT_BYTES + 1], usize::MAX), Err(Bypass::ReceiptBudget));
    }

    #[test]
    fn capture_rejects_ranges_overflow_duplicates_and_owns_completed_record() {
        let bytes = serde_json::to_vec(&wire()).unwrap();
        for (pointer,len) in [(-1,1),(0,-1),(i32::MAX,i32::MAX),(1,bytes.len() as i32)] {
            let mut capture = Capture::new(MAX_RECEIPT_BYTES);
            assert_ne!(capture.receive(&bytes,pointer,len),0);
            assert!(capture.finish(true, 0).is_err());
        }
        assert_eq!(Capture::new(MAX_RECEIPT_BYTES).finish(true, 0), Err(Bypass::MissingReceipt));
        let mut memory = vec![0;8]; memory.extend_from_slice(&bytes);
        let mut capture = Capture::new(MAX_RECEIPT_BYTES);
        assert_eq!(capture.receive(&memory,8,bytes.len() as i32),0);
        memory.fill(0);
        assert_eq!(capture.finish(true, 0),read(wire()));
        let mut capture = Capture::new(MAX_RECEIPT_BYTES);
        assert_eq!(capture.receive(&bytes,0,bytes.len() as i32),0);
        assert_eq!(capture.receive(&bytes,0,bytes.len() as i32),1);
        assert_eq!(capture.finish(true, 0),Err(Bypass::DuplicateReceipt));
        let mut capture = Capture::new(0);
        assert_eq!(capture.receive(&bytes,0,bytes.len() as i32),1);
        assert_eq!(capture.finish(true, 0),Err(Bypass::ReceiptBudget));
        let mut capture = Capture::new(MAX_RECEIPT_BYTES);
        assert_eq!(capture.receive(&[0xff],0,1),0, "capture copies without parsing inside Clang");
        assert_eq!(capture.finish(true,0),Err(Bypass::MalformedReceipt));
        let mut capture = Capture::new(MAX_RECEIPT_BYTES);
        assert_eq!(capture.receive(&[0xff],0,1),0);
        assert_eq!(capture.finish(false,0),Err(Bypass::CompilerFailure));
        let mut capture = Capture::new(MAX_RECEIPT_BYTES);
        assert_eq!(capture.receive(&[0xff],0,1),0);
        assert_eq!(capture.finish(true,1),Err(Bypass::Diagnostics));
    }

    #[test]
    fn counter_is_deterministic_but_expanded_temporal_and_unsupported_constructs_bypass() {
        let index = index(&files(), &["/a.cpp","/b.cpp"], None).unwrap();
        assert!(key(&index,&read(wire()).unwrap()).is_ok());
        for builtin in ["__DATE__","__TIME__","__TIMESTAMP__"] {
            let mut value=wire();value["volatileBuiltins"]=json!([builtin]);
            assert_eq!(key(&index,&read(value).unwrap()),Err(Bypass::TemporalExpansion));
        }
        for construct in ["asm","embed","has-embed","modules","remap","chained-pch"] {
            let mut value=wire();value["unsupported"]=json!([construct]);
            assert_eq!(key(&index,&read(value).unwrap()),Err(Bypass::UnsupportedConstruct));
        }
    }

    #[test]
    fn compiler_failure_or_any_diagnostic_is_never_admitted() {
        let index = index(&files(), &["/a.cpp"], None).unwrap();
        let receipt=read(wire()).unwrap();let args=index.environment.arguments("/a.cpp",None);
        assert_eq!(index.validate(&receipt,"/a.cpp",&args,false,0),Err(Bypass::CompilerFailure));
        assert_eq!(index.validate(&receipt,"/a.cpp",&args,true,1),Err(Bypass::Diagnostics));
    }

    #[test]
    fn preflight_rejects_alternate_action_configuration_environment_and_response_arguments() {
        let index = index(&files(), &["/a.cpp"], None).unwrap();
        let normal=index.environment.arguments("/a.cpp",None);
        for arg in ["@/args.rsp","-load","-ivfsoverlay","-fmodules","-fsyntax-only","--config=/config","-I/tmp"] {
            let mut args=normal.clone();args.push(arg.into());
            assert_eq!(index.base_key("/a.cpp",&args),Err(Bypass::UnsupportedInvocation));
        }
        let mut args=normal.clone();args[0]="-cc1as".into();
        assert_eq!(index.base_key("/a.cpp",&args),Err(Bypass::UnsupportedInvocation));
        let mut index=index;index.environment.variables.insert("CPATH".into(),"/tmp".into());
        assert_eq!(index.base_key("/a.cpp",&normal),Err(Bypass::UnsupportedInvocation));
        let sources=vec!["/a.cpp".into()];
        let mut args=Environment::link_arguments(&sources,&[]);args.push("-o/alternative".into());
        assert!(matches!(InputIndex::new(Environment::fixed(),identity(),vec![],files().iter().map(|(p,b)|(p.clone(),b.as_slice())),vec![],sources,None,args),Err(Bypass::UnsupportedInvocation)));
    }

    #[test]
    fn source_only_edit_reuses_unaffected_key_until_cross_tu_dependency_is_observed() {
        let mut files=files();let receipt=read(wire()).unwrap();
        let before=index(&files,&["/a.cpp","/b.cpp"],None).unwrap();
        files.insert("/b.cpp".into(),b"int c;".to_vec());
        let after=index(&files,&["/a.cpp","/b.cpp"],None).unwrap();
        assert_eq!(key(&before,&receipt),key(&after,&receipt));
        let mut value=wire();value["dependencies"].as_array_mut().unwrap().push(json!("/b.cpp"));
        let cross=read(value).unwrap();
        assert_ne!(key(&before,&cross),key(&after,&cross));
        files.insert("/a.cpp".into(),b"int z;".to_vec());
        assert_ne!(key(&before,&receipt),key(&index(&files,&["/a.cpp","/b.cpp"],None).unwrap(),&receipt));
    }

    #[test]
    fn namespace_order_resources_and_shadowing_invalidate_broad_identity() {
        let original=files();let receipt=read(wire()).unwrap();
        let before=index(&original,&["/a.cpp","/b.cpp"],None).unwrap();let baseline=key(&before,&receipt).unwrap();
        for (path,bytes) in [("/common.h",b"#define N 2".as_slice()),("/image.bin",b"change".as_slice()),("/unused.txt",b"new".as_slice()),("/include/stdio.h",b"shadow".as_slice())] {
            let mut files=original.clone();files.insert(path.into(),bytes.to_vec());
            assert_ne!(baseline,key(&index(&files,&["/a.cpp","/b.cpp"],None).unwrap(),&receipt).unwrap());
        }
        let mut removed=original.clone();removed.remove("/image.bin");
        assert_ne!(baseline,key(&index(&removed,&["/a.cpp","/b.cpp"],None).unwrap(),&receipt).unwrap());
        assert_ne!(baseline,key(&index(&original,&["/b.cpp","/a.cpp"],None).unwrap(),&receipt).unwrap());
        let mut changed=index(&original,&["/a.cpp","/b.cpp"],None).unwrap();changed.toolchain.compiler="d".repeat(64);
        assert_ne!(baseline,key(&changed,&receipt).unwrap());
        let sources=vec!["/a.cpp".into(),"/b.cpp".into()];
        let directory_added=InputIndex::new(Environment::fixed(),identity(),
            vec!["/include/stdio.h".into(),"/include/c++/v1/vector".into()], original.iter().map(|(p,b)|(p.clone(),b.as_slice())),
            vec!["/empty".into()], sources.clone(), None, Environment::link_arguments(&sources,&[])).unwrap();
        assert_ne!(baseline,key(&directory_added,&receipt).unwrap(),"empty directories are part of the accessible namespace");
    }

    #[test]
    fn pch_requires_exact_bytes_parsed_metadata_and_complete_closure() {
        let mut files=files();files.insert("/std.pch".into(),vec![1,2,3]);
        let before=index(&files,&["/a.cpp","/b.cpp"],Some("/std.pch")).unwrap();
        assert_eq!(key(&before,&read(wire()).unwrap()),Err(Bypass::PchMetadata));
        let mut value=wire();value["importedPchPaths"]=json!(["/std.pch"]);value["dependencies"].as_array_mut().unwrap().push(json!("/std.pch"));
        let receipt=read(value.clone()).unwrap();assert!(key(&before,&receipt).is_ok());
        value["pchMetadataIndependent"]=json!(false);
        assert_eq!(key(&before,&read(value).unwrap()),Err(Bypass::PchMetadata));
        files.insert("/std.pch".into(),vec![1,2,4]);
        assert_ne!(key(&before,&receipt),key(&index(&files,&["/a.cpp","/b.cpp"],Some("/std.pch")).unwrap(),&receipt));
        let no_pch=index(&files,&["/a.cpp","/b.cpp"],None).unwrap();
        assert_eq!(key(&no_pch,&receipt),Err(Bypass::PchMetadata));
    }

    #[test]
    fn output_or_directory_collisions_are_ineligible_before_lookup() {
        for path in ["/a.cpp.o","/a.cpp-deadbeef.o.tmp","/b.cpp.o","/main.wasm","/main-deadbeef.wasm.tmp","/tmp/input.h","/a.cpp/child","/modules.idx"] {
            let mut files=files();files.insert(path.into(),b"owned".to_vec());
            assert!(index(&files,&["/a.cpp","/b.cpp"],None).is_err(),"{path}");
        }
    }

    #[test]
    fn missing_wrong_source_and_generated_dependencies_bypass() {
        let index=index(&files(),&["/a.cpp","/b.cpp"],None).unwrap();
        for dependency in ["/missing.h","/a.cpp.o","/b.cpp.o","/tmp/x","/a.cpp-abcd.o.tmp"] {
            let mut value=wire();value["dependencies"].as_array_mut().unwrap().push(json!(dependency));
            assert_eq!(key(&index,&read(value).unwrap()),Err(Bypass::DependencyClosure));
        }
        let mut value=wire();value["source"]=json!("/b.cpp");
        assert_eq!(key(&index,&read(value).unwrap()),Err(Bypass::DependencyClosure));
        let mut value=wire();value["dependencies"]=json!(["/include/stdio.h"]);
        assert_eq!(key(&index,&read(value).unwrap()),Err(Bypass::DependencyClosure));
    }

    #[test]
    fn include_probe_domain_covers_negative_relative_and_temporary_output_candidates() {
        let index=index(&files(),&["/a.cpp","/b.cpp"],None).unwrap();
        for name in ["a.cpp.o","./b.cpp.o","a.cpp-123.o.tmp","/main.wasm","/tmp/no-file","/proc/self/fd","/dev/stdin","../../outside","bad\\name"] {
            let mut value=wire();value["includeProbes"]=json!([{"name":name,"angled":true,"includer":"/a.cpp","found":null}]);
            assert_eq!(key(&index,&read(value).unwrap()),Err(Bypass::ProbeDomain),"{name}");
        }
        let mut value=wire();value["includeProbes"]=json!([{"name":"absent.h","angled":false,"includer":"/a.cpp","found":null}]);
        assert!(key(&index,&read(value).unwrap()).is_ok());
    }

    #[test]
    fn positive_probe_requires_physical_includer_and_found_dependency_in_candidate_domain() {
        let index=index(&files(),&["/a.cpp","/b.cpp"],None).unwrap();
        let mut value=wire();value["includeProbes"]=json!([{"name":"stdio.h","angled":true,"includer":"/a.cpp","found":"/include/stdio.h"}]);
        assert!(key(&index,&read(value.clone()).unwrap()).is_ok());
        value["includeProbes"][0]["includer"]=json!("/b.cpp");
        assert_eq!(key(&index,&read(value).unwrap()),Err(Bypass::ProbeDomain));
        for (name,found) in [("b.cpp","/b.cpp"),("wrong.h","/include/stdio.h")] {
            let mut value=wire();value["includeProbes"]=json!([{"name":name,"angled":false,"includer":"/a.cpp","found":found}]);
            assert_eq!(key(&index,&read(value).unwrap()),Err(Bypass::ProbeDomain));
        }
    }
}

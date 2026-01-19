use wasmer::RuntimeError;
use wasmer_wasix::{
    WasiEnvBuilder,
    virtual_fs::{FileSystem, mem_fs},
};

pub struct Execution {
    fs: mem_fs::FileSystem,
}

pub struct Step<'a> {
    exec: &'a mut Execution,
    builder: WasiEnvBuilder,
    binary: Option<String>,
    sysroot: Option<String>,
    union_fs: Option<Box<dyn FileSystem>>,
}

impl Execution {
    pub fn new() -> Self {
        Self {
            fs: mem_fs::FileSystem::default(),
        }
    }

    pub fn step<'a>(&'a mut self, name: &str) -> Step<'a> {
        Step {
            exec: self,
            builder: WasiEnvBuilder::new(name),
            binary: None,
            sysroot: None,
            union_fs: None,
        }
    }
}

impl<'a> Step<'a> {
    /// Sets the binary to be executed for this step.
    ///
    /// If it starts with a "/", it is treated as an absolute path in the current filesystem.
    pub fn binary(mut self, url_or_path: &str) -> Self {
        self.binary = Some(String::from(url_or_path));
        self
    }

    /// Sets the sysroot to be used for this step.
    ///
    /// This should be a URL to a tarball which will be injected into the root of the filesystem.
    pub fn sysroot(mut self, url: &str) -> Self {
        self.sysroot = Some(String::from(url));
        self
    }

    /// Allows unioning a custom filesystem into this step's filesystem.
    ///
    /// The unioned filesystem will be layered on top of the sysroot, if any,
    /// potentially overwriting files in the sysroot if there are conflicts.
    pub fn fs(mut self, fs: Box<dyn FileSystem>) -> Self {
        self.union_fs = Some(fs);
        self
    }

    /// Adds arguments to be passed to argv.
    /// The program name is not included here.
    pub fn args<I, Arg>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = Arg>,
        Arg: AsRef<[u8]>,
    {
        self.builder.add_args(args);
        self
    }

    pub async fn run(self) -> Result<(), RuntimeError> {
        // TODO: This is just placeholder for now
        // let fs = std::mem::replace(&mut self.exec.fs, mem_fs::FileSystem::default());
        self.builder.fs(Box::new(self.exec.fs.clone()));
        Ok(())
    }
}

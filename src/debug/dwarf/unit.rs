use std::{
    num::NonZeroU64,
    path::{Path, PathBuf},
};

use crate::{
    debug::dwarf::{DerefContext, Die, Dwarf},
    types::GlobalAddress,
    util::weak_error,
};

use super::R;
use gimli::{Reader, UnitHeader};

#[derive(Debug, Clone)]
pub struct UnitProperties {
    /// The index of this unit among all units in the dwarf output
    index: usize,
    /// The index of the first location in this unit in the global locations list
    loc_offset: usize,
}

#[derive(Debug)]
pub struct Unit {
    /// Provides direct access to `gimli`
    unit: gimli::Unit<R>,
    properties: UnitProperties,
    files: Vec<PathBuf>,
    /// Information about the lines in this unit.
    /// Each of these is theoretically a breakable program statement
    /// (whether it actually is depends on if instrumentation code was generated)
    lines: Vec<LineRow>,
}

#[derive(PartialEq, Debug, Clone)]
#[repr(Rust, packed)]
pub struct LineRow {
    /// PC address within code segment
    address: GlobalAddress,
    /// Index of corresponding file within this unit
    file_index: usize,
    /// Line number within file (one-indexed)
    line: usize,
    /// Column number (0 is left edge)
    column: usize,
}

impl LineRow {
    #[inline]
    pub fn address(&self) -> GlobalAddress {
        self.address
    }

    #[inline]
    pub fn line(&self) -> usize {
        self.line
    }

    #[inline]
    pub fn column(&self) -> usize {
        self.column
    }
}

pub struct Location<'a> {
    pub unit: &'a Unit,
    pub line: &'a LineRow,
    pub file: &'a Path,
}

impl<'a> std::ops::Deref for Location<'a> {
    type Target = LineRow;

    fn deref(&self) -> &Self::Target {
        &self.line
    }
}

impl std::ops::Deref for Unit {
    type Target = gimli::Unit<R>;

    fn deref(&self) -> &Self::Target {
        &self.unit
    }
}

impl Unit {
    pub fn clone(&self, dwarf: &gimli::Dwarf<R>) -> Self {
        let unit = {
            dwarf
                .unit(self.unit.header.clone())
                .expect("clone unit should not fail")
        };

        Self {
            unit,
            properties: self.properties.clone(),
            files: self.files.clone(),
            lines: self.lines.clone(),
        }
    }

    /// Gets the root DIE for this unit
    pub fn root<'a>(&'a self, dwarf: &'a Dwarf) -> Option<Die<'a>> {
        let mut entries = self.unit.entries();
        weak_error!(entries.next_entry())?;
        let die = weak_error!(entries.current().ok_or(gimli::Error::MissingUnitDie))?.clone();
        Some(Die::new(DerefContext::new(dwarf, self), die))
    }

    pub fn unit(&self) -> &gimli::Unit<R> {
        &self.unit
    }

    pub fn index(&self) -> usize {
        self.properties.index
    }

    pub fn locations(&self) -> impl Iterator<Item = Location<'_>> {
        self.lines.iter().map(|l| Location {
            unit: self,
            line: l,
            file: &self.files[l.file_index as usize],
        })
    }

    pub fn location_at(&self, index: usize) -> Option<Location<'_>> {
        let local_index = index.checked_sub(self.properties.loc_offset)?;
        self.lines.get(local_index).map(|line| Location {
            unit: self,
            line,
            file: &self.files[line.file_index as usize],
        })
    }
}

pub struct UnitParser<'a> {
    dwarf: &'a gimli::Dwarf<R>,
    unit_index: usize,
    loc_index: usize,
}

impl<'a> UnitParser<'a> {
    pub fn new(dwarf: &'a gimli::Dwarf<R>) -> Self {
        UnitParser {
            dwarf,
            unit_index: 0,
            loc_index: 0,
        }
    }

    pub fn parse(&mut self, header: UnitHeader<R>) -> Option<Unit> {
        let unit = weak_error!(self.dwarf.unit(header))?;

        let mut files = vec![];
        let mut lines = vec![];
        if let Some(ref lp) = unit.line_program {
            let mut rows = lp.clone().rows();
            lines = weak_error!(parse_lines(&mut rows))?;
            files = weak_error!(parse_files(self.dwarf, &unit, &rows))?;
        }

        let index = self.unit_index;
        self.unit_index += 1;

        let loc_offset = self.loc_index;
        self.loc_index += lines.len();

        Some(Unit {
            properties: UnitProperties { index, loc_offset },
            unit,
            files,
            lines,
        })
    }
}

fn parse_lines(
    rows: &mut gimli::LineRows<R, gimli::IncompleteLineProgram<R>>,
) -> gimli::Result<Vec<LineRow>> {
    let mut lines = vec![];
    while let Some((_, line_row)) = rows.next_row()? {
        let column = match line_row.column() {
            gimli::ColumnType::LeftEdge => 0,
            gimli::ColumnType::Column(x) => x.get(),
        };

        if !line_row.is_stmt() {
            continue;
        }

        lines.push(LineRow {
            address: line_row.address().into(),
            file_index: line_row.file_index() as usize,
            line: line_row.line().map(NonZeroU64::get).unwrap_or(0) as usize,
            column: column as usize,
        })
    }

    lines.shrink_to_fit();
    Ok(lines)
}

fn parse_files(
    dwarf: &gimli::Dwarf<R>,
    unit: &gimli::Unit<R>,
    rows: &gimli::LineRows<R, gimli::IncompleteLineProgram<R>>,
) -> gimli::Result<Vec<PathBuf>> {
    let mut files = vec![];
    let header = rows.header();
    match header.file(0) {
        Some(file) => files.push(render_file_path(unit, file, header, dwarf)?),
        None => files.push(PathBuf::default()),
    }
    let mut index = 1;
    while let Some(file) = header.file(index) {
        files.push(render_file_path(unit, file, header, dwarf)?);
        index += 1;
    }

    files.shrink_to_fit();
    Ok(files)
}

fn render_file_path(
    dw_unit: &gimli::Unit<R>,
    file: &gimli::FileEntry<R>,
    header: &gimli::LineProgramHeader<R>,
    sections: &gimli::Dwarf<R>,
) -> gimli::Result<PathBuf> {
    let mut path = if let Some(ref comp_dir) = dw_unit.comp_dir {
        PathBuf::from(comp_dir.to_string_lossy()?.as_ref())
    } else {
        PathBuf::new()
    };

    if file.directory_index() != 0
        && let Some(directory) = file.directory(header)
    {
        path.push(
            sections
                .attr_string(dw_unit, directory)?
                .to_string_lossy()?
                .as_ref(),
        );
    }

    path.push(
        sections
            .attr_string(dw_unit, file.path_name())?
            .to_string_lossy()?
            .as_ref(),
    );

    Ok(path)
}

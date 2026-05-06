use std::{num::NonZeroU64, path::PathBuf};

use crate::{
    debug::dwarf::{DerefContext, Die, Dwarf},
    types::GlobalAddress,
    util::weak_error,
};

use super::R;
use gimli::{Reader, UnitHeader};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct UnitProperties {
    /// The index of this unit among all units in the dwarf output
    index: usize,
    /// The index of the first file in this unit in the global locations list
    file_offset: usize,
}

#[derive(Debug)]
pub struct Unit {
    /// Provides direct access to `gimli`
    unit: gimli::Unit<R>,
    properties: UnitProperties,
    files: Vec<PathBuf>,
}

#[derive(PartialEq, Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Location {
    /// Address within code segment
    pub address: GlobalAddress,
    /// Index of the corresponding file.
    /// Use [Dwarf::file_at] to get the associated [PathBuf].
    pub file_index: usize,
    /// Line number within file (one-indexed)
    pub line: usize,
    /// Column number (one-indexed)
    pub column: usize,
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

    pub fn locations(&self) -> impl Iterator<Item = Location> {
        let Some(line_program) = self.unit.line_program.clone() else {
            return Vec::new().into_iter();
        };

        let mut rows = line_program.rows();
        weak_error!(parse_lines(self.properties.file_offset, &mut rows))
            .unwrap_or_default()
            .into_iter()
    }

    pub fn file_at(&self, index: usize) -> Option<&PathBuf> {
        let local_index = index.checked_sub(self.properties.file_offset)?;
        self.files.get(local_index)
    }
}

pub struct UnitParser<'a> {
    dwarf: &'a gimli::Dwarf<R>,
    unit_index: usize,
    file_index: usize,
}

impl<'a> UnitParser<'a> {
    pub fn new(dwarf: &'a gimli::Dwarf<R>) -> Self {
        UnitParser {
            dwarf,
            unit_index: 0,
            file_index: 0,
        }
    }

    pub fn parse(&mut self, header: UnitHeader<R>) -> Option<Unit> {
        let unit = weak_error!(self.dwarf.unit(header))?;

        let mut files = vec![];
        if let Some(ref lp) = unit.line_program {
            let rows = lp.clone().rows();
            files = weak_error!(parse_files(self.dwarf, &unit, &rows))?;
        }

        let index = self.unit_index;
        self.unit_index += 1;

        let file_offset = self.file_index;
        self.file_index += files.len();

        Some(Unit {
            properties: UnitProperties { index, file_offset },
            unit,
            files,
        })
    }
}

fn parse_lines(
    file_offset: usize,
    rows: &mut gimli::LineRows<R, gimli::IncompleteLineProgram<R>>,
) -> gimli::Result<Vec<Location>> {
    let mut lines = vec![];
    while let Some((_, line_row)) = rows.next_row()? {
        let column = match line_row.column() {
            gimli::ColumnType::LeftEdge => 1,
            gimli::ColumnType::Column(x) => x.get(),
        };

        if !line_row.is_stmt() {
            continue;
        }

        lines.push(Location {
            address: line_row.address().into(),
            file_index: file_offset + line_row.file_index() as usize,
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

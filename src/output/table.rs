use std::cmp::max;
use std::env;
use std::ops::Deref;
#[cfg(unix)]
use std::sync::{Mutex, MutexGuard};

use datetime::TimeZone;
use zoneinfo_compiled::{CompiledData, Result as TZResult};

use lazy_static::lazy_static;
use log::*;
#[cfg(unix)]
use users::UsersCache;

use crate::fs::{File, fields as f};
use crate::fs::feature::git::GitCache;
use crate::output::cell::TextCell;
use crate::output::render::TimeRender;
use crate::output::time::TimeFormat;
use crate::theme::Theme;


/// Options for displaying a table.
#[derive(PartialEq, Eq, Debug)]
pub struct Options {
    pub size_format: SizeFormat,
    pub time_format: TimeFormat,
    pub user_format: UserFormat,
    pub columns: Columns,
}

/// Extra columns to display in the table.
#[allow(clippy::struct_excessive_bools)]
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub struct Columns {

    /// At least one of these timestamps will be shown.
    pub time_types: TimeTypes,

    // The rest are just on/off
    pub inode: bool,
    pub links: bool,
    pub blocks: bool,
    pub group: bool,
    pub git: bool,
    pub octal: bool,

    // Defaults to true:
    pub permissions: bool,
    pub filesize: bool,
    pub user: bool,

    pub path: bool,
}

impl Columns {
    pub fn collect(&self, actually_enable_git: bool) -> Vec<Column> {
        let mut columns = Vec::with_capacity(4);

        if self.path {
            columns.push(Column::Path);
        }

        if self.inode {
            #[cfg(unix)]
            columns.push(Column::Inode);
        }

        if self.octal {
            #[cfg(unix)]
            columns.push(Column::Octal);
        }

        if self.permissions {
            columns.push(Column::Permissions);
        }

        if self.links {
            #[cfg(unix)]
            columns.push(Column::HardLinks);
        }

        if self.filesize {
            columns.push(Column::FileSize);
        }

        if self.blocks {
            #[cfg(unix)]
            columns.push(Column::Blocks);
        }

        if self.user {
            #[cfg(unix)]
            columns.push(Column::User);
        }

        if self.group {
            #[cfg(unix)]
            columns.push(Column::Group);
        }

        if self.time_types.modified {
            columns.push(Column::Timestamp(TimeType::Modified));
        }

        if self.time_types.changed {
            columns.push(Column::Timestamp(TimeType::Changed));
        }

        if self.time_types.created {
            columns.push(Column::Timestamp(TimeType::Created));
        }

        if self.time_types.accessed {
            columns.push(Column::Timestamp(TimeType::Accessed));
        }

        if self.git && actually_enable_git {
            columns.push(Column::GitStatus);
        }

        columns
    }
}


/// A table contains these.
#[derive(Debug, Copy, Clone)]
pub enum Column {
    Path,
    Permissions,
    FileSize,
    Timestamp(TimeType),
    #[cfg(unix)]
    Blocks,
    #[cfg(unix)]
    User,
    #[cfg(unix)]
    Group,
    #[cfg(unix)]
    HardLinks,
    #[cfg(unix)]
    Inode,
    GitStatus,
    #[cfg(unix)]
    Octal,
}

/// Each column can pick its own **Alignment**. Usually, numbers are
/// right-aligned, and text is left-aligned.
#[derive(Copy, Clone)]
pub enum Alignment {
    Left,
    Right,
}

impl Column {

    /// Get the alignment this column should use.
    #[cfg(unix)]
    pub fn alignment(self) -> Alignment {
        match self {
            Self::Path       |
            Self::FileSize   |
            Self::HardLinks  |
            Self::Inode      |
            Self::Blocks     |
            Self::GitStatus  => Alignment::Right,
            _                => Alignment::Left,
        }
    }

    #[cfg(windows)]
    pub fn alignment(&self) -> Alignment {
        match self {
            Self::Path       |
            Self::FileSize   |
            Self::GitStatus  => Alignment::Right,
            _                => Alignment::Left,
        }
    }

    /// Get the text that should be printed at the top, when the user elects
    /// to have a header row printed.
    pub fn header(self) -> &'static str {
        match self {
            Self::Path          => "Path",
            #[cfg(unix)]
            Self::Permissions   => "Permissions",
            #[cfg(windows)]
            Self::Permissions   => "Mode",
            Self::FileSize      => "Size",
            Self::Timestamp(t)  => t.header(),
            #[cfg(unix)]
            Self::Blocks        => "Blocks",
            #[cfg(unix)]
            Self::User          => "User",
            #[cfg(unix)]
            Self::Group         => "Group",
            #[cfg(unix)]
            Self::HardLinks     => "Links",
            #[cfg(unix)]
            Self::Inode         => "inode",
            Self::GitStatus     => "Git",
            #[cfg(unix)]
            Self::Octal         => "Octal",
        }
    }
}


/// Formatting options for file sizes.
#[allow(clippy::enum_variant_names)]
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum SizeFormat {

    /// Format the file size using **decimal** prefixes, such as “kilo”,
    /// “mega”, or “giga”.
    DecimalBytes,

    /// Format the file size using **binary** prefixes, such as “kibi”,
    /// “mebi”, or “gibi”.
    BinaryBytes,

    /// Do no formatting and just display the size as a number of bytes.
    JustBytes,
}

/// Formatting options for user and group.
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum UserFormat {
    /// The UID / GID
    Numeric,
    /// Show the name
    Name,
}

impl Default for SizeFormat {
    fn default() -> Self {
        Self::DecimalBytes
    }
}


/// The types of a file’s time fields. These three fields are standard
/// across most (all?) operating systems.
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
pub enum TimeType {

    /// The file’s modified time (`st_mtime`).
    Modified,

    /// The file’s changed time (`st_ctime`)
    Changed,

    /// The file’s accessed time (`st_atime`).
    Accessed,

    /// The file’s creation time (`btime` or `birthtime`).
    Created,
}

impl TimeType {

    /// Returns the text to use for a column’s heading in the columns output.
    pub fn header(self) -> &'static str {
        match self {
            Self::Modified  => "Date Modified",
            Self::Changed   => "Date Changed",
            Self::Accessed  => "Date Accessed",
            Self::Created   => "Date Created",
        }
    }
}


/// Fields for which of a file’s time fields should be displayed in the
/// columns output.
///
/// There should always be at least one of these — there’s no way to disable
/// the time columns entirely (yet).
#[derive(PartialEq, Eq, Debug, Copy, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct TimeTypes {
    pub modified: bool,
    pub changed:  bool,
    pub accessed: bool,
    pub created:  bool,
}

impl Default for TimeTypes {

    /// By default, display just the ‘modified’ time. This is the most
    /// common option, which is why it has this shorthand.
    fn default() -> Self {
        Self {
            modified: true,
            changed:  false,
            accessed: false,
            created:  false,
        }
    }
}


/// The **environment** struct contains any data that could change between
/// running instances of exa, depending on the user’s computer’s configuration.
///
/// Any environment field should be able to be mocked up for test runs.
pub struct Environment {

    /// Localisation rules for formatting numbers.
    numeric: locale::Numeric,

    /// The computer’s current time zone. This gets used to determine how to
    /// offset files’ timestamps.
    tz: Option<TimeZone>,

    /// Mapping cache of user IDs to usernames.
    #[cfg(unix)]
    users: Mutex<UsersCache>,
}

impl Environment {
    #[cfg(unix)]
    pub fn lock_users(&self) -> MutexGuard<'_, UsersCache> {
        self.users.lock().unwrap()
    }

    fn load_all() -> Self {
        let tz = match determine_time_zone() {
            Ok(t) => {
                Some(t)
            }
            Err(ref e) => {
                println!("Unable to determine time zone: {}", e);
                None
            }
        };

        let numeric = locale::Numeric::load_user_locale()
                             .unwrap_or_else(|_| locale::Numeric::english());

        #[cfg(unix)]
        let users = Mutex::new(UsersCache::new());

        Self { numeric, tz, #[cfg(unix)] users }
    }
}

#[cfg(unix)]
fn determine_time_zone() -> TZResult<TimeZone> {
    if let Ok(file) = env::var("TZ") {
        TimeZone::from_file({
            if file.starts_with('/') {
                file
            } else {
                format!("/usr/share/zoneinfo/{}", {
                    if file.starts_with(':') {
                        file.replacen(':', "", 1)
                    } else {
                        file
                    }
                })
            }
        })
    } else {
        TimeZone::from_file("/etc/localtime")
    }
}

#[cfg(windows)]
fn determine_time_zone() -> TZResult<TimeZone> {
    use datetime::zone::{FixedTimespan, FixedTimespanSet, StaticTimeZone, TimeZoneSource};
    use std::borrow::Cow;

    Ok(TimeZone(TimeZoneSource::Static(&StaticTimeZone {
        name: "Unsupported",
        fixed_timespans: FixedTimespanSet {
            first: FixedTimespan {
                offset: 0,
                is_dst: false,
                name: Cow::Borrowed("ZONE_A"),
            },
            rest: &[(
                1206838800,
                FixedTimespan {
                    offset: 3600,
                    is_dst: false,
                    name: Cow::Borrowed("ZONE_B"),
                },
            )],
        },
    })))
}

lazy_static! {
    static ref ENVIRONMENT: Environment = Environment::load_all();
}


pub struct Table<'a> {
    columns: Vec<Column>,
    theme: &'a Theme,
    env: &'a Environment,
    widths: TableWidths,
    time_format: TimeFormat,
    size_format: SizeFormat,
    user_format: UserFormat,
    git: Option<&'a GitCache>,
}

#[derive(Clone)]
pub struct Row {
    cells: Vec<TextCell>,
}

impl<'a, 'f> Table<'a> {
    pub fn new(options: &'a Options, git: Option<&'a GitCache>, theme: &'a Theme) -> Table<'a> {
        let columns = options.columns.collect(git.is_some());
        let widths = TableWidths::zero(columns.len());
        let env = &*ENVIRONMENT;

        Table {
            theme,
            widths,
            columns,
            git,
            env,
            time_format: options.time_format,
            size_format: options.size_format,
            user_format: options.user_format,
        }
    }

    pub fn widths(&self) -> &TableWidths {
        &self.widths
    }

    pub fn header_row(&self) -> Row {
        let cells = self.columns.iter()
                        .map(|c| TextCell::paint_str(self.theme.ui.header, c.header()))
                        .collect();

        Row { cells }
    }

    pub fn row_for_file(&self, file: &File<'_>, xattrs: bool) -> Row {
        let cells = self.columns.iter()
                        .map(|c| self.display(file, *c, xattrs))
                        .collect();

        Row { cells }
    }

    pub fn add_widths(&mut self, row: &Row) {
        self.widths.add_widths(row)
    }

    fn permissions_plus(&self, file: &File<'_>, xattrs: bool) -> f::PermissionsPlus {
        f::PermissionsPlus {
            file_type: file.type_char(),
            #[cfg(unix)]
            permissions: file.permissions(),
            #[cfg(windows)]
            attributes: file.attributes(),
            xattrs,
        }
    }

    #[cfg(unix)]
    fn octal_permissions(&self, file: &File<'_>) -> f::OctalPermissions {
        f::OctalPermissions {
            permissions: file.permissions(),
        }
    }

    fn display(&self, file: &File<'_>, column: Column, xattrs: bool) -> TextCell {
        match column {
            Column::Path => {
                use ansi_term::Style;
                TextCell::paint(
                    Style::default(), 
                    file.path.display().to_string() + "!"
                )
            }
            Column::Permissions => {
                self.permissions_plus(file, xattrs).render(self.theme)
            }
            Column::FileSize => {
                file.size().render(self.theme, self.size_format, &self.env.numeric)
            }
            #[cfg(unix)]
            Column::HardLinks => {
                file.links().render(self.theme, &self.env.numeric)
            }
            #[cfg(unix)]
            Column::Inode => {
                file.inode().render(self.theme.ui.inode)
            }
            #[cfg(unix)]
            Column::Blocks => {
                file.blocks().render(self.theme)
            }
            #[cfg(unix)]
            Column::User => {
                file.user().render(self.theme, &*self.env.lock_users(), self.user_format)
            }
            #[cfg(unix)]
            Column::Group => {
                file.group().render(self.theme, &*self.env.lock_users(), self.user_format)
            }
            Column::GitStatus => {
                self.git_status(file).render(self.theme)
            }
            #[cfg(unix)]
            Column::Octal => {
                self.octal_permissions(file).render(self.theme.ui.octal)
            }

            Column::Timestamp(TimeType::Modified)  => {
                file.modified_time().render(self.theme.ui.date, &self.env.tz, self.time_format)
            }
            Column::Timestamp(TimeType::Changed)   => {
                file.changed_time().render(self.theme.ui.date, &self.env.tz, self.time_format)
            }
            Column::Timestamp(TimeType::Created)   => {
                file.created_time().render(self.theme.ui.date, &self.env.tz, self.time_format)
            }
            Column::Timestamp(TimeType::Accessed)  => {
                file.accessed_time().render(self.theme.ui.date, &self.env.tz, self.time_format)
            }
        }
    }

    fn git_status(&self, file: &File<'_>) -> f::Git {
        debug!("Getting Git status for file {:?}", file.path);

        self.git
            .map(|g| g.get(&file.path, file.is_directory()))
            .unwrap_or_default()
    }

    pub fn render(&self, row: Row) -> TextCell {
        let mut cell = TextCell::default();

        let iter = row.cells.into_iter()
                      .zip(self.widths.iter())
                      .enumerate();

        for (n, (this_cell, width)) in iter {
            let padding = width - *this_cell.width;

            match self.columns[n].alignment() {
                Alignment::Left => {
                    cell.append(this_cell);
                    cell.add_spaces(padding);
                }
                Alignment::Right => {
                    cell.add_spaces(padding);
                    cell.append(this_cell);
                }
            }

            cell.add_spaces(1);
        }

        cell
    }
}


pub struct TableWidths(Vec<usize>);

impl Deref for TableWidths {
    type Target = [usize];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl TableWidths {
    pub fn zero(count: usize) -> Self {
        Self(vec![0; count])
    }

    pub fn add_widths(&mut self, row: &Row) {
        for (old_width, cell) in self.0.iter_mut().zip(row.cells.iter()) {
            *old_width = max(*old_width, *cell.width);
        }
    }

    pub fn total(&self) -> usize {
        self.0.len() + self.0.iter().sum::<usize>()
    }
}

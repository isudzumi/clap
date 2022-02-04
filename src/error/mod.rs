//! Error reporting

// Std
use std::{
    borrow::Cow,
    convert::From,
    error,
    fmt::{self, Debug, Display, Formatter},
    io::{self, BufRead},
    result::Result as StdResult,
};

// Internal
use crate::{
    build::Arg,
    output::fmt::Colorizer,
    parse::features::suggestions,
    util::{color::ColorChoice, safe_exit, SUCCESS_CODE, USAGE_CODE},
    App, AppSettings,
};

mod context;
mod kind;

pub use context::ContextKind;
pub use context::ContextValue;
pub use kind::ErrorKind;

/// Short hand for [`Result`] type
///
/// [`Result`]: std::result::Result
pub type Result<T, E = Error> = StdResult<T, E>;

/// Command Line Argument Parser Error
///
/// See [`App::error`] to create an error.
///
/// [`App::error`]: crate::App::error
#[derive(Debug)]
pub struct Error {
    inner: Box<ErrorInner>,
    /// The type of error
    pub kind: ErrorKind,
    /// Additional information depending on the error kind, like values and argument names.
    /// Useful when you want to render an error of your own.
    pub info: Vec<String>,
}

#[derive(Debug)]
struct ErrorInner {
    kind: ErrorKind,
    context: Vec<(ContextKind, ContextValue)>,
    message: Option<Message>,
    source: Option<Box<dyn error::Error + Send + Sync>>,
    help_flag: Option<&'static str>,
    color_when: ColorChoice,
    wait_on_exit: bool,
    backtrace: Option<Backtrace>,
}

impl Error {
    /// Create an unformatted error
    ///
    /// This is for you need to pass the error up to
    /// a place that has access to the `App` at which point you can call [`Error::format`].
    ///
    /// Prefer [`App::error`] for generating errors.
    ///
    /// [`App::error`]: crate::App::error
    pub fn raw(kind: ErrorKind, message: impl std::fmt::Display) -> Self {
        Self::new(kind).set_message(message.to_string())
    }

    /// Format the existing message with the App's context
    #[must_use]
    pub fn format(mut self, app: &mut App) -> Self {
        app._build();
        let usage = app.render_usage();
        if let Some(message) = self.inner.message.as_mut() {
            message.format(app, usage);
        }
        self.with_app(app)
    }

    /// Type of error for programmatic processing
    pub fn kind(&self) -> ErrorKind {
        self.inner.kind
    }

    /// Additional information to further qualify the error
    pub fn context(&self) -> impl Iterator<Item = (ContextKind, &ContextValue)> {
        self.inner.context.iter().map(|(k, v)| (*k, v))
    }

    /// Should the message be written to `stdout` or not?
    #[inline]
    pub fn use_stderr(&self) -> bool {
        !matches!(
            self.kind(),
            ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
        )
    }

    /// Prints the error and exits.
    ///
    /// Depending on the error kind, this either prints to `stderr` and exits with a status of `2`
    /// or prints to `stdout` and exits with a status of `0`.
    pub fn exit(&self) -> ! {
        if self.use_stderr() {
            // Swallow broken pipe errors
            let _ = self.print();

            if self.inner.wait_on_exit {
                wlnerr!("\nPress [ENTER] / [RETURN] to continue...");
                let mut s = String::new();
                let i = io::stdin();
                i.lock().read_line(&mut s).unwrap();
            }

            safe_exit(USAGE_CODE);
        }

        // Swallow broken pipe errors
        let _ = self.print();
        safe_exit(SUCCESS_CODE)
    }

    /// Prints formatted and colored error to `stdout` or `stderr` according to its error kind
    ///
    /// # Example
    /// ```no_run
    /// use clap::App;
    ///
    /// match App::new("App").try_get_matches() {
    ///     Ok(matches) => {
    ///         // do_something
    ///     },
    ///     Err(err) => {
    ///         err.print().expect("Error writing Error");
    ///         // do_something
    ///     },
    /// };
    /// ```
    pub fn print(&self) -> io::Result<()> {
        self.formatted().print()
    }

    /// Deprecated, replaced with [`App::error`]
    ///
    /// [`App::error`]: crate::App::error
    #[deprecated(since = "3.0.0", note = "Replaced with `App::error`")]
    pub fn with_description(description: String, kind: ErrorKind) -> Self {
        Error::raw(kind, description)
    }

    fn new(kind: ErrorKind) -> Self {
        Self {
            inner: Box::new(ErrorInner {
                kind,
                context: Vec::new(),
                message: None,
                source: None,
                help_flag: None,
                color_when: ColorChoice::Never,
                wait_on_exit: false,
                backtrace: Backtrace::new(),
            }),
            kind,
            info: vec![],
        }
    }

    #[inline(never)]
    fn for_app(kind: ErrorKind, app: &App, colorizer: Colorizer, info: Vec<String>) -> Self {
        Self::new(kind)
            .set_message(colorizer)
            .with_app(app)
            .set_info(info)
    }

    fn with_app(self, app: &App) -> Self {
        self.set_wait_on_exit(app.settings.is_set(AppSettings::WaitOnError))
            .set_color(app.get_color())
            .set_help_flag(get_help_flag(app))
    }

    pub(crate) fn set_message(mut self, message: impl Into<Message>) -> Self {
        self.inner.message = Some(message.into());
        self
    }

    pub(crate) fn set_info(mut self, info: Vec<String>) -> Self {
        self.info = info;
        self
    }

    pub(crate) fn set_source(mut self, source: Box<dyn error::Error + Send + Sync>) -> Self {
        self.inner.source = Some(source);
        self
    }

    pub(crate) fn set_color(mut self, color_when: ColorChoice) -> Self {
        self.inner.color_when = color_when;
        self
    }

    pub(crate) fn set_help_flag(mut self, help_flag: Option<&'static str>) -> Self {
        self.inner.help_flag = help_flag;
        self
    }

    pub(crate) fn set_wait_on_exit(mut self, yes: bool) -> Self {
        self.inner.wait_on_exit = yes;
        self
    }

    /// Does not verify if `ContextKind` is already present
    pub(crate) fn insert_context_unchecked(
        mut self,
        kind: ContextKind,
        value: ContextValue,
    ) -> Self {
        self.inner.context.push((kind, value));
        self
    }

    /// Does not verify if `ContextKind` is already present
    pub(crate) fn extend_context_unchecked<const N: usize>(
        mut self,
        context: [(ContextKind, ContextValue); N],
    ) -> Self {
        self.inner.context.extend(context);
        self
    }

    fn get_context(&self, kind: ContextKind) -> Option<&ContextValue> {
        self.inner
            .context
            .iter()
            .find_map(|(k, v)| (*k == kind).then(|| v))
    }

    pub(crate) fn display_help(app: &App, colorizer: Colorizer) -> Self {
        Self::for_app(ErrorKind::DisplayHelp, app, colorizer, vec![])
    }

    pub(crate) fn display_help_error(app: &App, colorizer: Colorizer) -> Self {
        Self::for_app(
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand,
            app,
            colorizer,
            vec![],
        )
    }

    pub(crate) fn display_version(app: &App, colorizer: Colorizer) -> Self {
        Self::for_app(ErrorKind::DisplayVersion, app, colorizer, vec![])
    }

    pub(crate) fn argument_conflict(
        app: &App,
        arg: &Arg,
        mut others: Vec<String>,
        usage: String,
    ) -> Self {
        let info = others.clone();
        let others = match others.len() {
            0 => ContextValue::None,
            1 => ContextValue::Value(others.pop().unwrap()),
            _ => ContextValue::Values(others),
        };
        Self::new(ErrorKind::ArgumentConflict)
            .with_app(app)
            .set_info(info)
            .extend_context_unchecked([
                (
                    ContextKind::InvalidArg,
                    ContextValue::Value(arg.to_string()),
                ),
                (ContextKind::ValidArg, others),
                (ContextKind::Usage, ContextValue::Value(usage)),
            ])
    }

    pub(crate) fn empty_value(app: &App, good_vals: &[&str], arg: &Arg, usage: String) -> Self {
        let info = vec![arg.to_string()];
        let mut err = Self::new(ErrorKind::EmptyValue)
            .with_app(app)
            .set_info(info)
            .extend_context_unchecked([
                (
                    ContextKind::InvalidArg,
                    ContextValue::Value(arg.to_string()),
                ),
                (ContextKind::Usage, ContextValue::Value(usage)),
            ]);
        if !good_vals.is_empty() {
            err = err.insert_context_unchecked(
                ContextKind::ValidValue,
                ContextValue::Values(good_vals.iter().map(|s| (*s).to_owned()).collect()),
            );
        }
        err
    }

    pub(crate) fn no_equals(app: &App, arg: String, usage: String) -> Self {
        let info = vec![arg.to_string()];
        Self::new(ErrorKind::NoEquals)
            .with_app(app)
            .set_info(info)
            .extend_context_unchecked([
                (
                    ContextKind::InvalidArg,
                    ContextValue::Value(arg.to_string()),
                ),
                (ContextKind::Usage, ContextValue::Value(usage)),
            ])
    }

    pub(crate) fn invalid_value(
        app: &App,
        bad_val: String,
        good_vals: &[&str],
        arg: &Arg,
        usage: String,
    ) -> Self {
        let mut c = Colorizer::new(true, app.get_color());
        let suffix = suggestions::did_you_mean(&bad_val, good_vals.iter()).pop();
        let arg = arg.to_string();

        let good_vals: Vec<String> = good_vals
            .iter()
            .map(|&v| {
                if v.contains(char::is_whitespace) {
                    format!("{:?}", v)
                } else {
                    v.to_owned()
                }
            })
            .collect();

        start_error(&mut c);
        c.none("");
        c.warning(format!("{:?}", bad_val));
        c.none(" isn't a valid value for '");
        c.warning(&*arg);
        c.none("'\n\t[possible values: ");

        if let Some((last, elements)) = good_vals.split_last() {
            for v in elements {
                c.good(v);
                c.none(", ");
            }

            c.good(last);
        }

        c.none("]");

        if let Some(val) = suffix {
            c.none("\n\n\tDid you mean ");
            c.good(format!("{:?}", val));
            c.none("?");
        }

        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        let mut info = vec![arg, bad_val];
        info.extend(good_vals);

        Self::for_app(ErrorKind::InvalidValue, app, c, info)
    }

    pub(crate) fn invalid_subcommand(
        app: &App,
        subcmd: String,
        did_you_mean: String,
        name: String,
        usage: String,
    ) -> Self {
        let mut c = Colorizer::new(true, app.get_color());

        start_error(&mut c);
        c.none("The subcommand '");
        c.warning(&*subcmd);
        c.none("' wasn't recognized\n\n\tDid you mean ");
        c.good(did_you_mean);
        c.none("");
        c.none(format!(
            "?\n\nIf you believe you received this message in error, try re-running with '{} ",
            name
        ));
        c.good("--");
        c.none(format!(" {}'", subcmd));
        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(ErrorKind::InvalidSubcommand, app, c, vec![subcmd])
    }

    pub(crate) fn unrecognized_subcommand(app: &App, subcmd: String, name: String) -> Self {
        let mut c = Colorizer::new(true, app.get_color());

        start_error(&mut c);
        c.none(" The subcommand '");
        c.warning(&*subcmd);
        c.none("' wasn't recognized\n\n");
        c.warning("USAGE:");
        c.none(format!("\n    {} <subcommands>", name));
        try_help(&mut c, get_help_flag(app));

        Self::for_app(ErrorKind::UnrecognizedSubcommand, app, c, vec![subcmd])
    }

    pub(crate) fn missing_required_argument(
        app: &App,
        required: Vec<String>,
        usage: String,
    ) -> Self {
        let mut c = Colorizer::new(true, app.get_color());

        start_error(&mut c);
        c.none("The following required arguments were not provided:");

        for v in &required {
            c.none("\n    ");
            c.good(&**v);
        }

        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(ErrorKind::MissingRequiredArgument, app, c, required)
    }

    pub(crate) fn missing_subcommand(app: &App, name: String, usage: String) -> Self {
        let mut c = Colorizer::new(true, app.get_color());

        start_error(&mut c);
        c.none("'");
        c.warning(name);
        c.none("' requires a subcommand, but one was not provided");
        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(ErrorKind::MissingSubcommand, app, c, vec![])
    }

    pub(crate) fn invalid_utf8(app: &App, usage: String) -> Self {
        let mut c = Colorizer::new(true, app.get_color());

        start_error(&mut c);
        c.none("Invalid UTF-8 was detected in one or more arguments");
        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(ErrorKind::InvalidUtf8, app, c, vec![])
    }

    pub(crate) fn too_many_occurrences(
        app: &App,
        arg: &Arg,
        max_occurs: usize,
        curr_occurs: usize,
        usage: String,
    ) -> Self {
        let mut c = Colorizer::new(true, app.get_color());
        let were_provided = Error::singular_or_plural(curr_occurs);
        let arg = arg.to_string();
        let max_occurs = max_occurs.to_string();
        let curr_occurs = curr_occurs.to_string();

        start_error(&mut c);
        c.none("The argument '");
        c.warning(&*arg);
        c.none("' allows at most ");
        c.warning(&*max_occurs);
        c.none(" occurrences, but ");
        c.warning(&*curr_occurs);
        c.none(were_provided);
        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(
            ErrorKind::TooManyOccurrences,
            app,
            c,
            vec![arg, curr_occurs, max_occurs],
        )
    }

    pub(crate) fn too_many_values(app: &App, val: String, arg: String, usage: String) -> Self {
        let mut c = Colorizer::new(true, app.get_color());

        start_error(&mut c);
        c.none("The value '");
        c.warning(&*val);
        c.none("' was provided to '");
        c.warning(&*arg);
        c.none("' but it wasn't expecting any more values");
        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(ErrorKind::TooManyValues, app, c, vec![arg, val])
    }

    pub(crate) fn too_few_values(
        app: &App,
        arg: &Arg,
        min_vals: usize,
        curr_vals: usize,
        usage: String,
    ) -> Self {
        let mut c = Colorizer::new(true, app.get_color());
        let were_provided = Error::singular_or_plural(curr_vals);
        let arg = arg.to_string();
        let min_vals = min_vals.to_string();
        let curr_vals = curr_vals.to_string();

        start_error(&mut c);
        c.none("The argument '");
        c.warning(&*arg);
        c.none("' requires at least ");
        c.warning(&*min_vals);
        c.none(" values, but only ");
        c.warning(&*curr_vals);
        c.none(were_provided);
        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(
            ErrorKind::TooFewValues,
            app,
            c,
            vec![arg, curr_vals, min_vals],
        )
    }

    pub(crate) fn value_validation(
        app: &App,
        arg: String,
        val: String,
        err: Box<dyn error::Error + Send + Sync>,
    ) -> Self {
        let mut err =
            Self::value_validation_with_color(arg, val, err, app.get_color()).with_app(app);
        match err.inner.message.as_mut() {
            Some(Message::Formatted(c)) => try_help(c, get_help_flag(app)),
            _ => {
                unreachable!("`value_validation_with_color` only deals in formatted errors")
            }
        }
        err
    }

    pub(crate) fn value_validation_without_app(
        arg: String,
        val: String,
        err: Box<dyn error::Error + Send + Sync>,
    ) -> Self {
        let mut err = Self::value_validation_with_color(arg, val, err, ColorChoice::Never);
        match err.inner.message.as_mut() {
            Some(Message::Formatted(c)) => {
                c.none("\n");
            }
            _ => {
                unreachable!("`value_validation_with_color` only deals in formatted errors")
            }
        }
        err
    }

    fn value_validation_with_color(
        arg: String,
        val: String,
        err: Box<dyn error::Error + Send + Sync>,
        color: ColorChoice,
    ) -> Self {
        let mut c = Colorizer::new(true, color);

        start_error(&mut c);
        c.none("Invalid value");

        c.none(" for '");
        c.warning(&*arg);
        c.none("'");

        c.none(format!(": {}", err));

        Self::new(ErrorKind::ValueValidation)
            .set_message(c)
            .set_info(vec![arg, val, err.to_string()])
            .set_source(err)
    }

    pub(crate) fn wrong_number_of_values(
        app: &App,
        arg: &Arg,
        num_vals: usize,
        curr_vals: usize,
        usage: String,
    ) -> Self {
        let mut c = Colorizer::new(true, app.get_color());
        let were_provided = Error::singular_or_plural(curr_vals);
        let arg = arg.to_string();
        let num_vals = num_vals.to_string();
        let curr_vals = curr_vals.to_string();

        start_error(&mut c);
        c.none("The argument '");
        c.warning(&*arg);
        c.none("' requires ");
        c.warning(&*num_vals);
        c.none(" values, but ");
        c.warning(&*curr_vals);
        c.none(were_provided);
        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(
            ErrorKind::WrongNumberOfValues,
            app,
            c,
            vec![arg, curr_vals, num_vals],
        )
    }

    pub(crate) fn unexpected_multiple_usage(app: &App, arg: &Arg, usage: String) -> Self {
        let mut c = Colorizer::new(true, app.get_color());
        let arg = arg.to_string();

        start_error(&mut c);
        c.none("The argument '");
        c.warning(&*arg);
        c.none("' was provided more than once, but cannot be used multiple times");
        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(ErrorKind::UnexpectedMultipleUsage, app, c, vec![arg])
    }

    pub(crate) fn unknown_argument(
        app: &App,
        arg: String,
        did_you_mean: Option<(String, Option<String>)>,
        usage: String,
    ) -> Self {
        let mut c = Colorizer::new(true, app.get_color());

        start_error(&mut c);
        c.none("Found argument '");
        c.warning(&*arg);
        c.none("' which wasn't expected, or isn't valid in this context");

        if let Some((flag, subcmd)) = did_you_mean {
            let flag = format!("--{}", flag);
            c.none("\n\n\tDid you mean ");

            if let Some(subcmd) = subcmd {
                c.none("to put '");
                c.good(flag);
                c.none("' after the subcommand '");
                c.good(subcmd);
                c.none("'?");
            } else {
                c.none("'");
                c.good(flag);
                c.none("'?");
            }
        }

        // If the user wants to supply things like `--a-flag` or `-b` as a value,
        // suggest `--` for disambiguation.
        if arg.starts_with('-') {
            c.none(format!(
                "\n\n\tIf you tried to supply `{}` as a value rather than a flag, use `-- {}`",
                arg, arg
            ));
        }

        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(ErrorKind::UnknownArgument, app, c, vec![arg])
    }

    pub(crate) fn unnecessary_double_dash(app: &App, arg: String, usage: String) -> Self {
        let mut c = Colorizer::new(true, app.get_color());

        start_error(&mut c);
        c.none("Found argument '");
        c.warning(&*arg);
        c.none("' which wasn't expected, or isn't valid in this context");

        c.none(format!(
            "\n\n\tIf you tried to supply `{}` as a subcommand, remove the '--' before it.",
            arg
        ));
        put_usage(&mut c, usage);
        try_help(&mut c, get_help_flag(app));

        Self::for_app(ErrorKind::UnknownArgument, app, c, vec![arg])
    }

    pub(crate) fn argument_not_found_auto(arg: String) -> Self {
        let mut c = Colorizer::new(true, ColorChoice::Never);

        start_error(&mut c);
        c.none("The argument '");
        c.warning(&*arg);
        c.none("' wasn't found\n");

        Self::new(ErrorKind::ArgumentNotFound)
            .set_message(c)
            .set_info(vec![arg])
    }

    fn formatted(&self) -> Cow<'_, Colorizer> {
        if let Some(message) = self.inner.message.as_ref() {
            message.formatted()
        } else {
            let mut c = Colorizer::new(self.use_stderr(), self.inner.color_when);

            start_error(&mut c);

            match self.kind() {
                ErrorKind::ArgumentConflict => {
                    let invalid = self.get_context(ContextKind::InvalidArg);
                    let valid = self.get_context(ContextKind::ValidArg);
                    match (invalid, valid) {
                        (Some(ContextValue::Value(invalid)), Some(valid)) => {
                            c.none("The argument '");
                            c.warning(invalid);
                            c.none("' cannot be used with");

                            match valid {
                                ContextValue::Values(values) => {
                                    c.none(":");
                                    for v in values {
                                        c.none("\n    ");
                                        c.warning(&**v);
                                    }
                                }
                                ContextValue::Value(value) => {
                                    c.none(" '");
                                    c.warning(value);
                                    c.none("'");
                                }
                                _ => {
                                    c.none(" one or more of the other specified arguments");
                                }
                            }
                        }
                        (_, _) => {
                            c.none(self.kind().as_str().unwrap());
                        }
                    }
                }
                ErrorKind::EmptyValue => {
                    let invalid = self.get_context(ContextKind::InvalidArg);
                    match invalid {
                        Some(ContextValue::Value(invalid)) => {
                            c.none("The argument '");
                            c.warning(invalid);
                            c.none("' requires a value but none was supplied");
                        }
                        _ => {
                            c.none(self.kind().as_str().unwrap());
                        }
                    }

                    let possible_values = self.get_context(ContextKind::ValidValue);
                    if let Some(ContextValue::Values(possible_values)) = possible_values {
                        c.none("\n\t[possible values: ");
                        if let Some((last, elements)) = possible_values.split_last() {
                            for v in elements {
                                c.good(escape(v));
                                c.none(", ");
                            }
                            c.good(escape(last));
                        }
                        c.none("]");
                    }
                }
                ErrorKind::NoEquals => {
                    let invalid = self.get_context(ContextKind::InvalidArg);
                    match invalid {
                        Some(ContextValue::Value(invalid)) => {
                            c.none("Equal sign is needed when assigning values to '");
                            c.warning(invalid);
                            c.none("'.");
                        }
                        _ => {
                            c.none(self.kind().as_str().unwrap());
                        }
                    }
                }
                ErrorKind::InvalidValue
                | ErrorKind::UnknownArgument
                | ErrorKind::InvalidSubcommand
                | ErrorKind::UnrecognizedSubcommand
                | ErrorKind::ValueValidation
                | ErrorKind::TooManyValues
                | ErrorKind::TooFewValues
                | ErrorKind::TooManyOccurrences
                | ErrorKind::WrongNumberOfValues
                | ErrorKind::MissingRequiredArgument
                | ErrorKind::MissingSubcommand
                | ErrorKind::UnexpectedMultipleUsage
                | ErrorKind::InvalidUtf8
                | ErrorKind::DisplayHelp
                | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
                | ErrorKind::DisplayVersion
                | ErrorKind::ArgumentNotFound
                | ErrorKind::Io
                | ErrorKind::Format => {
                    if let Some(msg) = self.kind().as_str() {
                        c.none(msg.to_owned());
                    } else if let Some(source) = self.inner.source.as_ref() {
                        c.none(source.to_string());
                    } else {
                        c.none("Unknown cause");
                    }
                }
            }

            let usage = self.get_context(ContextKind::Usage);
            if let Some(ContextValue::Value(usage)) = usage {
                put_usage(&mut c, usage);
            }

            try_help(&mut c, self.inner.help_flag);

            Cow::Owned(c)
        }
    }

    /// Returns the singular or plural form on the verb to be based on the argument's value.
    fn singular_or_plural(n: usize) -> &'static str {
        if n > 1 {
            " were provided"
        } else {
            " was provided"
        }
    }
}

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Error::raw(ErrorKind::Io, e)
    }
}

impl From<fmt::Error> for Error {
    fn from(e: fmt::Error) -> Self {
        Error::raw(ErrorKind::Format, e)
    }
}

impl error::Error for Error {
    #[allow(trivial_casts)]
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.inner.source.as_ref().map(|e| e.as_ref() as _)
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        // Assuming `self.message` already has a trailing newline, from `try_help` or similar
        write!(f, "{}", self.formatted())?;
        if let Some(backtrace) = self.inner.backtrace.as_ref() {
            writeln!(f)?;
            writeln!(f, "Backtrace:")?;
            writeln!(f, "{}", backtrace)?;
        }
        Ok(())
    }
}

fn start_error(c: &mut Colorizer) {
    c.error("error:");
    c.none(" ");
}

fn put_usage(c: &mut Colorizer, usage: impl Into<String>) {
    c.none("\n\n");
    c.none(usage);
}

fn get_help_flag(app: &App) -> Option<&'static str> {
    if !app.settings.is_set(AppSettings::DisableHelpFlag) {
        Some("--help")
    } else if app.has_subcommands() && !app.settings.is_set(AppSettings::DisableHelpSubcommand) {
        Some("help")
    } else {
        None
    }
}

fn try_help(c: &mut Colorizer, help: Option<&str>) {
    if let Some(help) = help {
        c.none("\n\nFor more information try ");
        c.good(help);
        c.none("\n");
    } else {
        c.none("\n");
    }
}

fn escape(s: impl AsRef<str>) -> String {
    let s = s.as_ref();
    if s.contains(char::is_whitespace) {
        format!("{:?}", s)
    } else {
        s.to_owned()
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Message {
    Raw(String),
    Formatted(Colorizer),
}

impl Message {
    fn format(&mut self, app: &App, usage: String) {
        match self {
            Message::Raw(s) => {
                let mut c = Colorizer::new(true, app.get_color());

                let mut message = String::new();
                std::mem::swap(s, &mut message);
                start_error(&mut c);
                c.none(message);
                put_usage(&mut c, usage);
                try_help(&mut c, get_help_flag(app));
                *self = Self::Formatted(c);
            }
            Message::Formatted(_) => {}
        }
    }

    fn formatted(&self) -> Cow<Colorizer> {
        match self {
            Message::Raw(s) => {
                let mut c = Colorizer::new(true, ColorChoice::Never);
                start_error(&mut c);
                c.none(s);
                Cow::Owned(c)
            }
            Message::Formatted(c) => Cow::Borrowed(c),
        }
    }
}

impl From<String> for Message {
    fn from(inner: String) -> Self {
        Self::Raw(inner)
    }
}

impl From<Colorizer> for Message {
    fn from(inner: Colorizer) -> Self {
        Self::Formatted(inner)
    }
}

#[cfg(feature = "debug")]
#[derive(Debug)]
struct Backtrace(backtrace::Backtrace);

#[cfg(feature = "debug")]
impl Backtrace {
    fn new() -> Option<Self> {
        Some(Self(backtrace::Backtrace::new()))
    }
}

#[cfg(feature = "debug")]
impl Display for Backtrace {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        // `backtrace::Backtrace` uses `Debug` instead of `Display`
        write!(f, "{:?}", self.0)
    }
}

#[cfg(not(feature = "debug"))]
#[derive(Debug)]
struct Backtrace;

#[cfg(not(feature = "debug"))]
impl Backtrace {
    fn new() -> Option<Self> {
        None
    }
}

#[cfg(not(feature = "debug"))]
impl Display for Backtrace {
    fn fmt(&self, _: &mut Formatter) -> fmt::Result {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// Check `clap::Error` impls Send and Sync.
    mod clap_error_impl_send_sync {
        use crate::Error;
        trait Foo: std::error::Error + Send + Sync + 'static {}
        impl Foo for Error {}
    }
}

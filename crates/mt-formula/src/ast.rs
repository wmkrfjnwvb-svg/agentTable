//! The syntax tree of a formula and the table of built-in functions.
//!
//! A formula is stored as its tree, with fields referenced by id. Renaming a field therefore
//! never breaks a formula; the text people see is printed from the tree with the current
//! names ([`Expr::to_text`]).

use mt_core::decimal::{canonical_string, Decimal};
use mt_core::id::FieldId;
use serde::{Deserialize, Serialize};

/// Deepest nesting of a formula.
pub const MAX_DEPTH: usize = 64;
/// Maximum number of nodes in a formula.
pub const MAX_NODES: usize = 2_000;
/// Maximum number of characters in a formula's text.
pub const MAX_SOURCE_CHARS: usize = 10_000;

/// An operator with one operand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UnaryOp {
    /// `-x`.
    Negate,
    /// `x%`: a hundredth of `x`.
    Percent,
}

/// An operator with two operands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinaryOp {
    /// `+`.
    Add,
    /// `-`.
    Subtract,
    /// `*`.
    Multiply,
    /// `/`.
    Divide,
    /// `&`: joins two texts.
    Concat,
    /// `=`.
    Equal,
    /// `<>`.
    NotEqual,
    /// `<`.
    Less,
    /// `<=`.
    LessOrEqual,
    /// `>`.
    Greater,
    /// `>=`.
    GreaterOrEqual,
}

impl BinaryOp {
    /// The operator as written.
    #[must_use]
    pub const fn symbol(self) -> &'static str {
        match self {
            BinaryOp::Add => "+",
            BinaryOp::Subtract => "-",
            BinaryOp::Multiply => "*",
            BinaryOp::Divide => "/",
            BinaryOp::Concat => "&",
            BinaryOp::Equal => "=",
            BinaryOp::NotEqual => "<>",
            BinaryOp::Less => "<",
            BinaryOp::LessOrEqual => "<=",
            BinaryOp::Greater => ">",
            BinaryOp::GreaterOrEqual => ">=",
        }
    }

    /// Binding strength: comparisons bind loosest, then `&`, then `+ -`, then `* /`.
    #[must_use]
    pub const fn precedence(self) -> u8 {
        match self {
            BinaryOp::Equal
            | BinaryOp::NotEqual
            | BinaryOp::Less
            | BinaryOp::LessOrEqual
            | BinaryOp::Greater
            | BinaryOp::GreaterOrEqual => 1,
            BinaryOp::Concat => 2,
            BinaryOp::Add | BinaryOp::Subtract => 3,
            BinaryOp::Multiply | BinaryOp::Divide => 4,
        }
    }

    /// Whether the operator is a comparison.
    #[must_use]
    pub const fn is_comparison(self) -> bool {
        self.precedence() == 1
    }
}

/// Binding strength of the unary minus (tighter than every binary operator).
pub(crate) const UNARY_PRECEDENCE: u8 = 5;

macro_rules! function_groups {
    ( $(
        $(#[$group_meta:meta])*
        $group:ident($group_enum:ident) {
            $( $(#[$meta:meta])* $variant:ident => $name:literal, $min:literal, $max:expr; )+
        }
    )+ ) => {
        $(
            $(#[$group_meta])*
            #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
            pub enum $group_enum {
                $( $(#[$meta])* $variant, )+
            }

            impl $group_enum {
                /// Every function of the group, in declaration order.
                pub const ALL: &'static [$group_enum] = &[ $( $group_enum::$variant, )+ ];

                /// The name as written in formulas (upper case).
                #[must_use]
                pub const fn name(self) -> &'static str {
                    match self { $( $group_enum::$variant => $name, )+ }
                }

                /// Fewest arguments accepted.
                #[must_use]
                pub const fn min_args(self) -> usize {
                    match self { $( $group_enum::$variant => $min, )+ }
                }

                /// Most arguments accepted (`None`: any number).
                #[must_use]
                pub const fn max_args(self) -> Option<usize> {
                    match self { $( $group_enum::$variant => $max, )+ }
                }
            }
        )+

        /// A built-in function. Text and serde form: its name (`"SUM"`).
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum Function {
            $( $(#[$group_meta])* $group($group_enum), )+
        }

        impl Function {
            /// Every function, group by group.
            #[must_use]
            pub fn all() -> Vec<Function> {
                let mut all = Vec::new();
                $( all.extend($group_enum::ALL.iter().copied().map(Function::$group)); )+
                all
            }

            /// The name as written in formulas (upper case).
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self { $( Function::$group(function) => function.name(), )+ }
            }

            /// Fewest arguments accepted.
            #[must_use]
            pub const fn min_args(self) -> usize {
                match self { $( Function::$group(function) => function.min_args(), )+ }
            }

            /// Most arguments accepted (`None`: any number).
            #[must_use]
            pub const fn max_args(self) -> Option<usize> {
                match self { $( Function::$group(function) => function.max_args(), )+ }
            }
        }
    };
}

function_groups! {
    /// Decisions and tests.
    Logic(LogicFn) {
        /// `IF(condition, then, [else])`.
        If => "IF", 2, Some(3);
        /// `IFS(condition1, value1, condition2, value2, …)`: the value of the first true condition.
        Ifs => "IFS", 2, None;
        /// `SWITCH(value, case1, result1, …, [default])`.
        Switch => "SWITCH", 3, None;
        /// `AND(a, b, …)`.
        And => "AND", 1, None;
        /// `OR(a, b, …)`.
        Or => "OR", 1, None;
        /// `NOT(a)`.
        Not => "NOT", 1, Some(1);
        /// `XOR(a, b, …)`: true when an odd number of arguments are true.
        Xor => "XOR", 1, None;
        /// `IFERROR(value, fallback)`.
        IfError => "IFERROR", 2, Some(2);
        /// `IFBLANK(value, fallback)`.
        IfBlank => "IFBLANK", 2, Some(2);
        /// `ISBLANK(value)`.
        IsBlank => "ISBLANK", 1, Some(1);
        /// `ISERROR(value)`.
        IsError => "ISERROR", 1, Some(1);
        /// `ISNUMBER(value)`.
        IsNumber => "ISNUMBER", 1, Some(1);
        /// `ISTEXT(value)`.
        IsText => "ISTEXT", 1, Some(1);
        /// `BLANK()`: the empty value.
        Blank => "BLANK", 0, Some(0);
    }

    /// Calculations with numbers, amounts and durations.
    Number(NumberFn) {
        /// `SUM(a, b, …)`; lists are added up, empty values ignored.
        Sum => "SUM", 1, None;
        /// `AVERAGE(a, b, …)` of the values present.
        Average => "AVERAGE", 1, None;
        /// `MIN(a, b, …)`; also the earliest date.
        Min => "MIN", 1, None;
        /// `MAX(a, b, …)`; also the latest date.
        Max => "MAX", 1, None;
        /// `MEDIAN(a, b, …)`.
        Median => "MEDIAN", 1, None;
        /// `COUNT(a, b, …)`: how many numbers, amounts, durations and dates.
        Count => "COUNT", 1, None;
        /// `COUNTA(a, b, …)`: how many values that are not empty.
        CountA => "COUNTA", 1, None;
        /// `ABS(x)`.
        Abs => "ABS", 1, Some(1);
        /// `SIGN(x)`: -1, 0 or 1.
        Sign => "SIGN", 1, Some(1);
        /// `ROUND(x, [digits])`: half away from zero.
        Round => "ROUND", 1, Some(2);
        /// `ROUNDUP(x, [digits])`: away from zero.
        RoundUp => "ROUNDUP", 1, Some(2);
        /// `ROUNDDOWN(x, [digits])`: toward zero.
        RoundDown => "ROUNDDOWN", 1, Some(2);
        /// `INT(x)`: the largest whole number not above `x`.
        Int => "INT", 1, Some(1);
        /// `MOD(x, divisor)`: the remainder, with the sign of the divisor.
        Mod => "MOD", 2, Some(2);
        /// `POWER(x, n)` for a whole number `n`.
        Power => "POWER", 2, Some(2);
        /// `CEILING(x, [step])`: up to the next multiple of `step`.
        Ceiling => "CEILING", 1, Some(2);
        /// `FLOOR(x, [step])`: down to the previous multiple of `step`.
        Floor => "FLOOR", 1, Some(2);
    }

    /// Working with text.
    Text(TextFn) {
        /// `CONCAT(a, b, …)` (also written `CONCATENATE`).
        Concat => "CONCAT", 1, None;
        /// `TEXTJOIN(separator, ignore_empty, a, b, …)`.
        TextJoin => "TEXTJOIN", 3, None;
        /// `LEN(text)`: number of characters.
        Len => "LEN", 1, Some(1);
        /// `LEFT(text, [count])`.
        Left => "LEFT", 1, Some(2);
        /// `RIGHT(text, [count])`.
        Right => "RIGHT", 1, Some(2);
        /// `MID(text, start, count)`; `start` counts from 1.
        Mid => "MID", 3, Some(3);
        /// `UPPER(text)`.
        Upper => "UPPER", 1, Some(1);
        /// `LOWER(text)`.
        Lower => "LOWER", 1, Some(1);
        /// `TRIM(text)`: no spaces at the ends, single spaces inside.
        Trim => "TRIM", 1, Some(1);
        /// `SUBSTITUTE(text, old, new, [occurrence])`.
        Substitute => "SUBSTITUTE", 3, Some(4);
        /// `REPLACE(text, start, count, new)`.
        Replace => "REPLACE", 4, Some(4);
        /// `FIND(needle, text, [start])`: position, case-sensitive.
        Find => "FIND", 2, Some(3);
        /// `SEARCH(needle, text, [start])`: position, ignoring case.
        Search => "SEARCH", 2, Some(3);
        /// `REPT(text, times)`.
        Rept => "REPT", 2, Some(2);
        /// `VALUE(text)`: the number written in the text.
        Value => "VALUE", 1, Some(1);
        /// `TEXT(value, format)`.
        Text => "TEXT", 2, Some(2);
        /// `EXACT(a, b)`: equal including case.
        Exact => "EXACT", 2, Some(2);
        /// `CONTAINS(text_or_list, value)`.
        Contains => "CONTAINS", 2, Some(2);
    }

    /// Dates, times, working days and durations.
    Date(DateFn) {
        /// `TODAY()`.
        Today => "TODAY", 0, Some(0);
        /// `NOW()`.
        Now => "NOW", 0, Some(0);
        /// `DATE(year, month, day)`.
        Date => "DATE", 3, Some(3);
        /// `YEAR(date)`.
        Year => "YEAR", 1, Some(1);
        /// `MONTH(date)`.
        Month => "MONTH", 1, Some(1);
        /// `DAY(date)`.
        Day => "DAY", 1, Some(1);
        /// `HOUR(datetime)`.
        Hour => "HOUR", 1, Some(1);
        /// `MINUTE(datetime)`.
        Minute => "MINUTE", 1, Some(1);
        /// `WEEKDAY(date, [type])`: 1 = Sunday … 7 = Saturday; with type 2, 1 = Monday.
        Weekday => "WEEKDAY", 1, Some(2);
        /// `ISOWEEKNUM(date)`.
        IsoWeekNum => "ISOWEEKNUM", 1, Some(1);
        /// `DATEDIF(start, end, unit)` with unit `"D"`, `"M"` or `"Y"`.
        DateDif => "DATEDIF", 3, Some(3);
        /// `DATEADD(date, count, unit)` with unit `"days"`, `"weeks"`, `"months"` or `"years"`.
        DateAdd => "DATEADD", 3, Some(3);
        /// `EDATE(date, months)`.
        EDate => "EDATE", 2, Some(2);
        /// `EOMONTH(date, months)`: the last day of the month.
        EoMonth => "EOMONTH", 2, Some(2);
        /// `DAYS(end, start)`.
        Days => "DAYS", 2, Some(2);
        /// `NETWORKDAYS(start, end)`: working days, both ends included.
        NetworkDays => "NETWORKDAYS", 2, Some(2);
        /// `WORKDAY(start, days)`: the date that many working days later.
        Workday => "WORKDAY", 2, Some(2);
        /// `DURATION(hours, [minutes], [seconds])`.
        Duration => "DURATION", 1, Some(3);
        /// `TO_HOURS(duration)`.
        ToHours => "TO_HOURS", 1, Some(1);
        /// `TO_DAYS(duration)`.
        ToDays => "TO_DAYS", 1, Some(1);
    }

    /// Lists: the values of linked records and of multi-value fields.
    List(ListFn) {
        /// `COUNTIF(list, criterion)`.
        CountIf => "COUNTIF", 2, Some(2);
        /// `SUMIF(list, criterion, [sum_list])`.
        SumIf => "SUMIF", 2, Some(3);
        /// `AVERAGEIF(list, criterion, [average_list])`.
        AverageIf => "AVERAGEIF", 2, Some(3);
        /// `FILTER(list, condition)`; the condition uses `CurrentValue`.
        Filter => "FILTER", 2, Some(2);
        /// `UNIQUE(list)`.
        Unique => "UNIQUE", 1, Some(1);
        /// `FIRST(list)`.
        First => "FIRST", 1, Some(1);
        /// `LAST(list)`.
        Last => "LAST", 1, Some(1);
        /// `INDEX(list, position)`; positions count from 1.
        Index => "INDEX", 2, Some(2);
    }

    /// Reference data and settings.
    Lookup(LookupFn) {
        /// `LABEL(reference)`: the item's name.
        Label => "LABEL", 1, Some(1);
        /// `CODE(reference)`: the item's code.
        Code => "CODE", 1, Some(1);
        /// `ATTR(reference, "attribute")`: a custom attribute of the item.
        Attr => "ATTR", 2, Some(2);
        /// `PARAM("key")`: a parameter of the tenant, base or user.
        Param => "PARAM", 1, Some(1);
    }
}

impl Serialize for Function {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Function {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = <std::borrow::Cow<'de, str> as Deserialize>::deserialize(deserializer)?;
        Function::by_name(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown function `{name}`")))
    }
}

impl Function {
    /// The function with this name, ignoring case. `CONCATENATE` is another name for
    /// `CONCAT`.
    #[must_use]
    pub fn by_name(name: &str) -> Option<Function> {
        let upper = name.to_ascii_uppercase();
        if upper == "CONCATENATE" {
            return Some(Function::Text(TextFn::Concat));
        }
        Function::all().into_iter().find(|f| f.name() == upper)
    }

    /// What the function accepts, in words.
    #[must_use]
    pub fn arity_text(self) -> String {
        match (self.min_args(), self.max_args()) {
            (min, Some(max)) if min == max => min.to_string(),
            (min, Some(max)) => format!("{min} to {max}"),
            (min, None) => format!("at least {min}"),
        }
    }
}

/// A formula as a tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "snake_case")]
pub enum Expr {
    /// A number.
    Number(#[serde(with = "mt_core::decimal::serde_canonical")] Decimal),
    /// A text.
    Text(String),
    /// `TRUE` or `FALSE`.
    Bool(bool),
    /// The value of a field of the record: `{Amount}`.
    Field(FieldId),
    /// The values of a field of the records linked through a link field:
    /// `{Order lines}.{Amount}`.
    Through {
        /// The link field of this table.
        link: FieldId,
        /// The field of the linked table.
        field: FieldId,
    },
    /// The item under consideration inside `FILTER`, `COUNTIF`, `SUMIF` and `AVERAGEIF`.
    CurrentValue,
    /// An operator with one operand.
    Unary {
        /// The operator.
        op: UnaryOp,
        /// The operand.
        operand: Box<Expr>,
    },
    /// An operator with two operands.
    Binary {
        /// The operator.
        op: BinaryOp,
        /// The left operand.
        left: Box<Expr>,
        /// The right operand.
        right: Box<Expr>,
    },
    /// A function call.
    Call {
        /// The function.
        function: Function,
        /// The arguments.
        args: Vec<Expr>,
    },
}

/// The names to print a formula with.
pub trait Names {
    /// The current name of a field of the formula's table.
    fn field(&self, field: FieldId) -> Option<String>;

    /// The current name of a field of the table a link field points to.
    fn linked_field(&self, link: FieldId, field: FieldId) -> Option<String>;
}

fn braces(name: Option<String>, id: FieldId, out: &mut String) {
    out.push('{');
    match name {
        Some(name) => {
            for c in name.chars() {
                if matches!(c, '}' | '\\') {
                    out.push('\\');
                }
                out.push(c);
            }
        }
        // A field that no longer exists is printed by its id, which does not parse back as
        // a name: the formula visibly needs fixing.
        None => out.push_str(&format!("#{id}")),
    }
    out.push('}');
}

impl Expr {
    /// The children of this node, in order.
    fn children(&self) -> Vec<&Expr> {
        match self {
            Expr::Number(_)
            | Expr::Text(_)
            | Expr::Bool(_)
            | Expr::Field(_)
            | Expr::Through { .. }
            | Expr::CurrentValue => Vec::new(),
            Expr::Unary { operand, .. } => vec![operand],
            Expr::Binary { left, right, .. } => vec![left, right],
            Expr::Call { args, .. } => args.iter().collect(),
        }
    }

    /// Calls `visit` for this node and every node below it, parents first.
    pub fn walk(&self, visit: &mut dyn FnMut(&Expr)) {
        let mut stack = vec![self];
        while let Some(node) = stack.pop() {
            visit(node);
            let children = node.children();
            stack.extend(children.into_iter().rev());
        }
    }

    /// Number of nodes.
    #[must_use]
    pub fn node_count(&self) -> usize {
        let mut count = 0_usize;
        self.walk(&mut |_| count = count.saturating_add(1));
        count
    }

    /// Nesting depth (1 for a single value).
    #[must_use]
    pub fn depth(&self) -> usize {
        let mut deepest = 0_usize;
        let mut stack = vec![(self, 1_usize)];
        while let Some((node, level)) = stack.pop() {
            deepest = deepest.max(level);
            let below = level.saturating_add(1);
            stack.extend(node.children().into_iter().map(|child| (child, below)));
        }
        deepest
    }

    /// The formula as text, with the given field names and no more parentheses than needed.
    /// Parsing the text gives this tree back.
    #[must_use]
    pub fn to_text(&self, names: &dyn Names) -> String {
        let mut out = String::new();
        self.print(names, 0, &mut out);
        out
    }

    fn print(&self, names: &dyn Names, context: u8, out: &mut String) {
        match self {
            Expr::Number(n) => {
                // A negative literal in an operand position needs parentheses: `2 - (-3)`.
                if n.is_sign_negative() && !n.is_zero() && context > 0 {
                    out.push('(');
                    out.push_str(&canonical_string(n));
                    out.push(')');
                } else {
                    out.push_str(&canonical_string(n));
                }
            }
            Expr::Text(text) => {
                out.push('"');
                out.push_str(&text.replace('"', "\"\""));
                out.push('"');
            }
            Expr::Bool(true) => out.push_str("TRUE"),
            Expr::Bool(false) => out.push_str("FALSE"),
            Expr::Field(field) => braces(names.field(*field), *field, out),
            Expr::Through { link, field } => {
                braces(names.field(*link), *link, out);
                out.push('.');
                braces(names.linked_field(*link, *field), *field, out);
            }
            Expr::CurrentValue => out.push_str("CurrentValue"),
            Expr::Unary { op, operand } => match op {
                UnaryOp::Negate => {
                    let wrap = context > UNARY_PRECEDENCE;
                    if wrap {
                        out.push('(');
                    }
                    out.push('-');
                    operand.print(names, UNARY_PRECEDENCE, out);
                    if wrap {
                        out.push(')');
                    }
                }
                UnaryOp::Percent => {
                    // The operand of `%` is a primary expression.
                    operand.print(names, UNARY_PRECEDENCE.saturating_add(1), out);
                    out.push('%');
                }
            },
            Expr::Binary { op, left, right } => {
                let precedence = op.precedence();
                let wrap = context > precedence;
                if wrap {
                    out.push('(');
                }
                // Operators group to the left; comparisons do not chain, so both sides of a
                // comparison must bind tighter.
                let left_context = if op.is_comparison() {
                    precedence.saturating_add(1)
                } else {
                    precedence
                };
                left.print(names, left_context, out);
                out.push(' ');
                out.push_str(op.symbol());
                out.push(' ');
                right.print(names, precedence.saturating_add(1), out);
                if wrap {
                    out.push(')');
                }
            }
            Expr::Call { function, args } => {
                out.push_str(function.name());
                out.push('(');
                for (index, arg) in args.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    arg.print(names, 0, out);
                }
                out.push(')');
            }
        }
    }
}

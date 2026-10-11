// SPDX-License-Identifier: GPL-3.0-or-later
// Pinned Prometheus token identities; see NOTICE and LICENSE-APACHE-2.0.
#[allow(non_camel_case_types)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum TokenKind {
    EQL = 57346,
    BLANK = 57347,
    COLON = 57348,
    COMMA = 57349,
    COMMENT = 57350,
    DURATION = 57351,
    EOF = 57352,
    ERROR = 57353,
    IDENTIFIER = 57354,
    LEFT_BRACE = 57355,
    LEFT_BRACKET = 57356,
    LEFT_PAREN = 57357,
    OPEN_HIST = 57358,
    CLOSE_HIST = 57359,
    METRIC_IDENTIFIER = 57360,
    NUMBER = 57361,
    RIGHT_BRACE = 57362,
    RIGHT_BRACKET = 57363,
    RIGHT_PAREN = 57364,
    SEMICOLON = 57365,
    SPACE = 57366,
    STRING = 57367,
    TIMES = 57368,
    SUM_DESC = 57370,
    COUNT_DESC = 57371,
    SCHEMA_DESC = 57372,
    OFFSET_DESC = 57373,
    NEGATIVE_OFFSET_DESC = 57374,
    BUCKETS_DESC = 57375,
    NEGATIVE_BUCKETS_DESC = 57376,
    ZERO_BUCKET_DESC = 57377,
    ZERO_BUCKET_WIDTH_DESC = 57378,
    CUSTOM_VALUES_DESC = 57379,
    COUNTER_RESET_HINT_DESC = 57380,
    ADD = 57383,
    DIV = 57384,
    EQLC = 57385,
    EQL_REGEX = 57386,
    GTE = 57387,
    GTR = 57388,
    TRIM_UPPER = 57389,
    TRIM_LOWER = 57390,
    LAND = 57391,
    LOR = 57392,
    LSS = 57393,
    LTE = 57394,
    LUNLESS = 57395,
    MOD = 57396,
    MUL = 57397,
    NEQ = 57398,
    NEQ_REGEX = 57399,
    POW = 57400,
    SUB = 57401,
    AT = 57402,
    ATAN2 = 57403,
    AVG = 57406,
    BOTTOMK = 57407,
    COUNT = 57408,
    COUNT_VALUES = 57409,
    GROUP = 57410,
    MAX = 57411,
    MIN = 57412,
    QUANTILE = 57413,
    STDDEV = 57414,
    STDVAR = 57415,
    SUM = 57416,
    TOPK = 57417,
    LIMITK = 57418,
    LIMIT_RATIO = 57419,
    BOOL = 57422,
    BY = 57423,
    GROUP_LEFT = 57424,
    GROUP_RIGHT = 57425,
    FILL = 57426,
    FILL_LEFT = 57427,
    FILL_RIGHT = 57428,
    IGNORING = 57429,
    OFFSET = 57430,
    SMOOTHED = 57431,
    ANCHORED = 57432,
    ON = 57433,
    WITHOUT = 57434,
    START = 57437,
    END = 57438,
    STEP = 57439,
    RANGE = 57440,
    MAX_OF = 57441,
    MIN_OF = 57442,
    UNKNOWN_COUNTER_RESET = 57445,
    COUNTER_RESET = 57446,
    NOT_COUNTER_RESET = 57447,
    GAUGE_TYPE = 57448,
    START_METRIC = 57451,
    START_SERIES_DESCRIPTION = 57452,
    START_EXPRESSION = 57453,
    START_METRIC_SELECTOR = 57454,
}

impl TokenKind {
    pub const fn precedence(self) -> Option<u8> {
        match self {
            Self::LOR => Some(1),
            Self::LAND | Self::LUNLESS => Some(2),
            Self::EQLC
            | Self::GTE
            | Self::GTR
            | Self::LSS
            | Self::LTE
            | Self::NEQ
            | Self::TRIM_UPPER
            | Self::TRIM_LOWER => Some(3),
            Self::ADD | Self::SUB => Some(4),
            Self::MUL | Self::DIV | Self::MOD | Self::ATAN2 => Some(5),
            Self::POW => Some(6),
            _ => None,
        }
    }

    pub const fn is_operator(self) -> bool {
        (self as u32) > 57382 && (self as u32) < 57404
    }
    pub const fn is_aggregator(self) -> bool {
        (self as u32) > 57405 && (self as u32) < 57420
    }
    pub const fn is_keyword(self) -> bool {
        (self as u32) > 57421 && (self as u32) < 57435
    }
    pub const fn is_comparison(self) -> bool {
        matches!(
            self,
            Self::EQLC | Self::NEQ | Self::LTE | Self::LSS | Self::GTE | Self::GTR
        )
    }
    pub const fn is_set(self) -> bool {
        matches!(self, Self::LAND | Self::LOR | Self::LUNLESS)
    }
    pub const fn takes_parameter(self) -> bool {
        matches!(
            self,
            Self::TOPK
                | Self::BOTTOMK
                | Self::COUNT_VALUES
                | Self::QUANTILE
                | Self::LIMITK
                | Self::LIMIT_RATIO
        )
    }
    pub const fn is_experimental_aggregator(self) -> bool {
        matches!(self, Self::LIMITK | Self::LIMIT_RATIO)
    }
    pub const fn is_metric_identifier(self) -> bool {
        self.is_aggregator()
            || matches!(
                self,
                Self::BY
                    | Self::FILL
                    | Self::FILL_LEFT
                    | Self::FILL_RIGHT
                    | Self::IDENTIFIER
                    | Self::LAND
                    | Self::LOR
                    | Self::LUNLESS
                    | Self::METRIC_IDENTIFIER
                    | Self::OFFSET
                    | Self::WITHOUT
                    | Self::START
                    | Self::END
                    | Self::STEP
                    | Self::RANGE
                    | Self::ANCHORED
                    | Self::SMOOTHED
                    | Self::MAX_OF
                    | Self::MIN_OF
            )
    }
    pub const fn is_grouping_label(self) -> bool {
        self.is_metric_identifier()
            || matches!(
                self,
                Self::BOOL
                    | Self::GROUP_LEFT
                    | Self::GROUP_RIGHT
                    | Self::IGNORING
                    | Self::ON
                    | Self::ATAN2
            )
    }
    pub const fn is_function_identifier(self) -> bool {
        matches!(
            self,
            Self::IDENTIFIER
                | Self::START
                | Self::END
                | Self::STEP
                | Self::RANGE
                | Self::MAX_OF
                | Self::MIN_OF
        )
    }

    pub const fn text(self) -> Option<&'static str> {
        match self {
            Self::OPEN_HIST => Some("{{"),
            Self::CLOSE_HIST => Some("}}"),
            Self::LEFT_PAREN => Some("("),
            Self::RIGHT_PAREN => Some(")"),
            Self::LEFT_BRACE => Some("{"),
            Self::RIGHT_BRACE => Some("}"),
            Self::LEFT_BRACKET => Some("["),
            Self::RIGHT_BRACKET => Some("]"),
            Self::COMMA => Some(","),
            Self::EQL => Some("="),
            Self::COLON => Some(":"),
            Self::SEMICOLON => Some(";"),
            Self::BLANK => Some("_"),
            Self::TIMES => Some("x"),
            Self::SPACE => Some("<space>"),
            Self::SUB => Some("-"),
            Self::ADD => Some("+"),
            Self::MUL => Some("*"),
            Self::MOD => Some("%"),
            Self::DIV => Some("/"),
            Self::EQLC => Some("=="),
            Self::NEQ => Some("!="),
            Self::LTE => Some("<="),
            Self::LSS => Some("<"),
            Self::GTE => Some(">="),
            Self::GTR => Some(">"),
            Self::TRIM_UPPER => Some("</"),
            Self::TRIM_LOWER => Some(">/"),
            Self::EQL_REGEX => Some("=~"),
            Self::NEQ_REGEX => Some("!~"),
            Self::POW => Some("^"),
            Self::AT => Some("@"),
            Self::ATAN2 => Some("atan2"),
            Self::LAND => Some("and"),
            Self::LOR => Some("or"),
            Self::LUNLESS => Some("unless"),
            Self::SUM => Some("sum"),
            Self::AVG => Some("avg"),
            Self::COUNT => Some("count"),
            Self::MIN => Some("min"),
            Self::MAX => Some("max"),
            Self::GROUP => Some("group"),
            Self::STDDEV => Some("stddev"),
            Self::STDVAR => Some("stdvar"),
            Self::TOPK => Some("topk"),
            Self::BOTTOMK => Some("bottomk"),
            Self::COUNT_VALUES => Some("count_values"),
            Self::QUANTILE => Some("quantile"),
            Self::LIMITK => Some("limitk"),
            Self::LIMIT_RATIO => Some("limit_ratio"),
            Self::OFFSET => Some("offset"),
            Self::SMOOTHED => Some("smoothed"),
            Self::ANCHORED => Some("anchored"),
            Self::BY => Some("by"),
            Self::WITHOUT => Some("without"),
            Self::ON => Some("on"),
            Self::IGNORING => Some("ignoring"),
            Self::GROUP_LEFT => Some("group_left"),
            Self::GROUP_RIGHT => Some("group_right"),
            Self::FILL => Some("fill"),
            Self::FILL_LEFT => Some("fill_left"),
            Self::FILL_RIGHT => Some("fill_right"),
            Self::BOOL => Some("bool"),
            Self::START => Some("start"),
            Self::END => Some("end"),
            Self::STEP => Some("step"),
            Self::RANGE => Some("range"),
            Self::MAX_OF => Some("max_of"),
            Self::MIN_OF => Some("min_of"),
            _ => None,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::EQL => "EQL",
            Self::BLANK => "BLANK",
            Self::COLON => "COLON",
            Self::COMMA => "COMMA",
            Self::COMMENT => "COMMENT",
            Self::DURATION => "DURATION",
            Self::EOF => "EOF",
            Self::ERROR => "ERROR",
            Self::IDENTIFIER => "IDENTIFIER",
            Self::LEFT_BRACE => "LEFT_BRACE",
            Self::LEFT_BRACKET => "LEFT_BRACKET",
            Self::LEFT_PAREN => "LEFT_PAREN",
            Self::OPEN_HIST => "OPEN_HIST",
            Self::CLOSE_HIST => "CLOSE_HIST",
            Self::METRIC_IDENTIFIER => "METRIC_IDENTIFIER",
            Self::NUMBER => "NUMBER",
            Self::RIGHT_BRACE => "RIGHT_BRACE",
            Self::RIGHT_BRACKET => "RIGHT_BRACKET",
            Self::RIGHT_PAREN => "RIGHT_PAREN",
            Self::SEMICOLON => "SEMICOLON",
            Self::SPACE => "SPACE",
            Self::STRING => "STRING",
            Self::TIMES => "TIMES",
            Self::SUM_DESC => "SUM_DESC",
            Self::COUNT_DESC => "COUNT_DESC",
            Self::SCHEMA_DESC => "SCHEMA_DESC",
            Self::OFFSET_DESC => "OFFSET_DESC",
            Self::NEGATIVE_OFFSET_DESC => "NEGATIVE_OFFSET_DESC",
            Self::BUCKETS_DESC => "BUCKETS_DESC",
            Self::NEGATIVE_BUCKETS_DESC => "NEGATIVE_BUCKETS_DESC",
            Self::ZERO_BUCKET_DESC => "ZERO_BUCKET_DESC",
            Self::ZERO_BUCKET_WIDTH_DESC => "ZERO_BUCKET_WIDTH_DESC",
            Self::CUSTOM_VALUES_DESC => "CUSTOM_VALUES_DESC",
            Self::COUNTER_RESET_HINT_DESC => "COUNTER_RESET_HINT_DESC",
            Self::ADD => "ADD",
            Self::DIV => "DIV",
            Self::EQLC => "EQLC",
            Self::EQL_REGEX => "EQL_REGEX",
            Self::GTE => "GTE",
            Self::GTR => "GTR",
            Self::TRIM_UPPER => "TRIM_UPPER",
            Self::TRIM_LOWER => "TRIM_LOWER",
            Self::LAND => "LAND",
            Self::LOR => "LOR",
            Self::LSS => "LSS",
            Self::LTE => "LTE",
            Self::LUNLESS => "LUNLESS",
            Self::MOD => "MOD",
            Self::MUL => "MUL",
            Self::NEQ => "NEQ",
            Self::NEQ_REGEX => "NEQ_REGEX",
            Self::POW => "POW",
            Self::SUB => "SUB",
            Self::AT => "AT",
            Self::ATAN2 => "ATAN2",
            Self::AVG => "AVG",
            Self::BOTTOMK => "BOTTOMK",
            Self::COUNT => "COUNT",
            Self::COUNT_VALUES => "COUNT_VALUES",
            Self::GROUP => "GROUP",
            Self::MAX => "MAX",
            Self::MIN => "MIN",
            Self::QUANTILE => "QUANTILE",
            Self::STDDEV => "STDDEV",
            Self::STDVAR => "STDVAR",
            Self::SUM => "SUM",
            Self::TOPK => "TOPK",
            Self::LIMITK => "LIMITK",
            Self::LIMIT_RATIO => "LIMIT_RATIO",
            Self::BOOL => "BOOL",
            Self::BY => "BY",
            Self::GROUP_LEFT => "GROUP_LEFT",
            Self::GROUP_RIGHT => "GROUP_RIGHT",
            Self::FILL => "FILL",
            Self::FILL_LEFT => "FILL_LEFT",
            Self::FILL_RIGHT => "FILL_RIGHT",
            Self::IGNORING => "IGNORING",
            Self::OFFSET => "OFFSET",
            Self::SMOOTHED => "SMOOTHED",
            Self::ANCHORED => "ANCHORED",
            Self::ON => "ON",
            Self::WITHOUT => "WITHOUT",
            Self::START => "START",
            Self::END => "END",
            Self::STEP => "STEP",
            Self::RANGE => "RANGE",
            Self::MAX_OF => "MAX_OF",
            Self::MIN_OF => "MIN_OF",
            Self::UNKNOWN_COUNTER_RESET => "UNKNOWN_COUNTER_RESET",
            Self::COUNTER_RESET => "COUNTER_RESET",
            Self::NOT_COUNTER_RESET => "NOT_COUNTER_RESET",
            Self::GAUGE_TYPE => "GAUGE_TYPE",
            Self::START_METRIC => "START_METRIC",
            Self::START_SERIES_DESCRIPTION => "START_SERIES_DESCRIPTION",
            Self::START_EXPRESSION => "START_EXPRESSION",
            Self::START_METRIC_SELECTOR => "START_METRIC_SELECTOR",
        }
    }
}

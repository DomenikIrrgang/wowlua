//! What a front-end asks a session and what the session tells it (spec
//! debugger §18.12): one vocabulary for DAP, MCP and Studio. Sources are
//! paths relative to the package root (`src/core.wowlua`).

use serde::{Deserialize, Serialize};

/// A line breakpoint as set.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LineBreakpoint {
    pub line: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// `5` (the fifth hit), `>5`, `>=5`, `<5`, `<=5`, `==5`, `%5`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit_condition: Option<String>,
    /// Printed instead of stopping: `{expr}` parts evaluated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_message: Option<String>,
}

/// A function, API, event, script or timer breakpoint (§18.4).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FunctionBreakpoint {
    /// `Name`, `Table.name`, `Table:method`, `C_Timer.After`,
    /// `event:PLAYER_LOGIN`, `event:*`, `script:OnClick`,
    /// `script:MyFrame:OnUpdate`, `timer`.
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// A breakpoint's state after setting it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BreakpointState {
    pub id: u32,
    pub verified: bool,
    /// The line it took (a line without code moves to the next).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// One frame of a stack, innermost first.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackFrame {
    pub id: u32,
    pub name: String,
    /// `lua`, `native` or `dispatch`.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// The chunk name when the frame's code is not the package's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chunk: Option<String>,
    /// The file on disk of such code, when there is one (Blizzard's UI
    /// in the local cache, a `test.addons` folder's), absolute.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub line: u32,
    /// The function's taint: `secure`, or the addon.
    pub taint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub name: String,
    pub reference: u32,
    pub expensive: bool,
}

/// A value as shown.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(rename = "type")]
    pub ty: String,
    /// Non-zero when it has children (a table, a function's upvalues).
    pub reference: u32,
    /// `secure`, or the addon whose code wrote it.
    pub taint: String,
    /// A secret: `value` shows what it hides.
    pub secret: bool,
}

/// Where an evaluation happens.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum EvalContext {
    Watch,
    Repl,
    Hover,
}

/// Input as the user gives it (the engine's input functions).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum Input {
    /// A click on a frame (by name) or at a point (screen pixels, top
    /// left origin).
    Click {
        #[serde(default)]
        frame: Option<String>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
        #[serde(default = "left_button")]
        button: String,
    },
    Hover {
        #[serde(default)]
        frame: Option<String>,
        #[serde(default)]
        x: Option<f64>,
        #[serde(default)]
        y: Option<f64>,
    },
    Drag {
        from: String,
        to: String,
    },
    Type {
        text: String,
    },
    Key {
        key: String,
    },
    /// A chat line as the user types it: a slash command (`/wdemo`).
    Slash {
        text: String,
    },
}

fn left_button() -> String {
    "LeftButton".into()
}

/// A request to a session.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "camelCase")]
pub enum Request {
    SetBreakpoints {
        source: String,
        breakpoints: Vec<LineBreakpoint>,
    },
    SetFunctionBreakpoints {
        breakpoints: Vec<FunctionBreakpoint>,
    },
    /// `uncaught`, `all`, `taint`, `blocked`.
    SetExceptionBreakpoints {
        filters: Vec<String>,
    },
    /// The configuration is done: the session starts running.
    ConfigurationDone,
    Continue,
    Next,
    StepIn,
    StepOut,
    Pause,
    StackTrace,
    Scopes {
        frame: u32,
    },
    Variables {
        reference: u32,
    },
    SetVariable {
        reference: u32,
        name: String,
        value: String,
    },
    Evaluate {
        expression: String,
        #[serde(default)]
        frame: Option<u32>,
        #[serde(default = "repl")]
        context: EvalContext,
    },
    /// A PNG of the screen, or of one frame's rectangle.
    Screenshot {
        #[serde(default)]
        frame: Option<String>,
        #[serde(default)]
        scale: Option<f64>,
    },
    FrameTree {
        #[serde(default)]
        root: Option<String>,
        #[serde(default)]
        hidden: bool,
    },
    FindFrames {
        query: String,
    },
    Input {
        input: Input,
    },
    /// Moves time on (a `manual` session): seconds or frames.
    Advance {
        #[serde(default)]
        seconds: Option<f64>,
        #[serde(default)]
        frames: Option<u32>,
    },
    FireEvent {
        event: String,
        #[serde(default)]
        args: Vec<serde_json::Value>,
    },
    ProfileStart,
    ProfileStop,
    /// What was printed and raised since the last time asked.
    Output,
    Terminate,
}

fn repl() -> EvalContext {
    EvalContext::Repl
}

/// Why the session stopped.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Entry,
    Breakpoint,
    FunctionBreakpoint,
    Step,
    Pause,
    Exception,
}

impl StopReason {
    /// DAP's name.
    pub fn dap(self) -> &'static str {
        match self {
            StopReason::Entry => "entry",
            StopReason::Breakpoint => "breakpoint",
            StopReason::FunctionBreakpoint => "function breakpoint",
            StopReason::Step => "step",
            StopReason::Pause => "pause",
            StopReason::Exception => "exception",
        }
    }
}

/// A request's answer.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Response {
    Ok,
    Breakpoints {
        breakpoints: Vec<BreakpointState>,
    },
    StackTrace {
        frames: Vec<StackFrame>,
    },
    Scopes {
        scopes: Vec<Scope>,
    },
    Variables {
        variables: Vec<Variable>,
    },
    Value {
        variable: Variable,
    },
    /// PNG bytes, base64 encoded.
    Image {
        png_base64: String,
        width: u32,
        height: u32,
    },
    /// JSON data (a frame tree, frames found, a profile report).
    Data {
        data: serde_json::Value,
    },
    Output {
        lines: Vec<OutputLine>,
    },
}

/// Something the session tells on its own.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "camelCase")]
pub enum Event {
    /// The session is set up and waits for its configuration.
    Initialized,
    Stopped {
        reason: StopReason,
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        /// The breakpoints it stopped at.
        #[serde(default)]
        hit: Vec<u32>,
    },
    Continued,
    Output {
        line: OutputLine,
    },
    /// A breakpoint's state changed (its chunk loaded).
    Breakpoint {
        breakpoint: BreakpointState,
    },
    /// The session ended: a test file's result, the error that ended it.
    Terminated {
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        failed: bool,
    },
}

/// A line of output.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct OutputLine {
    /// `stdout` (printed), `stderr` (errors), `console` (the debugger's).
    pub category: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

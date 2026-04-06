use jaq_all::data;
use jaq_all::fmts::{self, Format};
use jaq_all::jaq_core::Filter;
use jaq_all::jaq_core::ValT as _;
use jaq_all::json::Val;
use jaq_all::load::FileReportsDisp;
use std::io::Cursor;

pub(crate) struct CompileError {
    pub(crate) exit_code: i32,
    pub(crate) message: String,
}

pub(crate) fn compile_filter(code: &str) -> Result<Filter<data::DataKind>, CompileError> {
    data::compile(code).map_err(|reports| {
        let message = reports
            .iter()
            .map(|report| FileReportsDisp::new(report).to_string())
            .collect::<Vec<_>>()
            .join("");
        let exit_code =
            if message.contains("undefined") || message.contains("wrong number of arguments") {
                3
            } else {
                5
            };
        CompileError { exit_code, message }
    })
}

pub(crate) fn collect_inputs(
    format: Format,
    contents: &[String],
    slurp: bool,
) -> Result<Vec<Val>, String> {
    let mut values = Vec::new();

    for content in contents {
        let cursor = Cursor::new(content.as_bytes());
        let parsed = fmts::read::read(format, cursor, content, false);
        for value in parsed {
            values.push(value.map_err(|err| err.to_string())?);
        }
    }

    if slurp {
        Ok(vec![values.into_iter().collect()])
    } else {
        Ok(values)
    }
}

pub(crate) fn run_filter(
    filter: &Filter<data::DataKind>,
    null_input: bool,
    inputs: Vec<Val>,
) -> Result<Vec<Val>, String> {
    let runner = data::Runner {
        null_input,
        ..Default::default()
    };
    let vars = Default::default();
    let mut output = Vec::new();

    data::run(
        &runner,
        filter,
        vars,
        inputs.into_iter().map(Ok::<Val, String>),
        |err| err,
        |value| {
            output.push(value.map_err(|err| err.to_string())?);
            Ok(())
        },
    )?;

    Ok(output)
}

pub(crate) fn make_writer(
    format: Format,
    compact: bool,
    join: bool,
    sort_keys: bool,
    indent: usize,
    use_tab: bool,
) -> fmts::write::Writer {
    let mut writer = fmts::write::Writer {
        format,
        join: true,
        ..Default::default()
    };
    writer.pp.sort_keys = sort_keys;
    writer.pp.sep_space = !compact;
    if !compact {
        writer.pp.indent = Some(if use_tab {
            "\t".to_string()
        } else {
            " ".repeat(indent.max(1))
        });
    }
    writer.join = join;
    writer
}

pub(crate) fn format_values(
    values: &[Val],
    format: Format,
    compact: bool,
    join: bool,
    sort_keys: bool,
    indent: usize,
    use_tab: bool,
) -> Result<String, String> {
    let separator = if join {
        ""
    } else if matches!(format, Format::Yaml) {
        "\n---\n"
    } else {
        "\n"
    };

    let mut rendered = Vec::with_capacity(values.len());
    for value in values {
        let writer = make_writer(format, compact, true, sort_keys, indent, use_tab);
        let mut buffer = Vec::new();
        fmts::write::write(&mut buffer, &writer, value).map_err(|err| err.to_string())?;
        rendered.push(String::from_utf8(buffer).map_err(|err| err.to_string())?);
    }

    let mut output = rendered.join(separator);
    if !join && !output.is_empty() {
        output.push('\n');
    }
    Ok(output)
}

pub(crate) fn last_truthy(values: &[Val]) -> Option<bool> {
    values.last().map(|value| value.as_bool())
}

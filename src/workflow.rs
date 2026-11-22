//! Sequential workflow pipelines: named steps, template substitution, child agents.

use std::collections::HashMap;
use std::io::Write as _;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;
use serde_json::json;

use crate::agent::{self, Config, EventSink};
use crate::session::Session;

const MAX_STEPS: usize = 8;

#[derive(Debug, Deserialize)]
pub struct Workflow {
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
pub struct Step {
    pub name: String,
    pub prompt: String,
}

pub async fn run(config: Config, path: &Path, workflow_session: &Session) -> Result<()> {
    let text = tokio::fs::read_to_string(path)
        .await
        .with_context(|| format!("reading {}", path.display()))?;
    let workflow: Workflow = serde_json::from_str(&text)
        .with_context(|| format!("parsing {} as a workflow JSON", path.display()))?;

    if workflow.steps.is_empty() {
        bail!("workflow has no steps");
    }
    if workflow.steps.len() > MAX_STEPS {
        bail!("workflow has {} steps; max is {MAX_STEPS}", workflow.steps.len());
    }
    if config.max_depth == 0 {
        bail!("workflows need --max-depth >= 1");
    }

    let mut outputs: HashMap<String, String> = HashMap::new();
    let mut previous = String::new();

    for (index, step) in workflow.steps.iter().enumerate() {
        let name = step.name.trim();
        if name.is_empty() {
            bail!("step {} has an empty name", index + 1);
        }
        if outputs.contains_key(name) {
            bail!("duplicate step name `{name}`");
        }

        let prompt = substitute(&step.prompt, &outputs, &previous);
        eprintln!("[workflow] step {}/{}: {name}", index + 1, workflow.steps.len());

        let child = Session::create(&config.data_dir, &config.model).await?;
        child
            .append(json!({
                "type": "workflow_step",
                "workflow": workflow_session.id,
                "index": index + 1,
                "name": name,
            }))
            .await?;

        let child_config = Config {
            depth: config.depth + 1,
            max_steps: config.max_steps.min(4),
            ..config.clone()
        };
        let mut sink = StepSink;
        let outcome = agent::run(child_config, &child, &prompt, Vec::new(), &mut sink)
            .await
            .with_context(|| format!("workflow step `{name}`"))?;
        sink.finish();
        let output = outcome.content.trim().to_string();

        workflow_session
            .append(json!({
                "type": "workflow_step_result",
                "name": name,
                "session": child.id,
                "output": output,
            }))
            .await?;

        previous = output.clone();
        outputs.insert(name.to_string(), output);
    }

    Ok(())
}

fn substitute(template: &str, outputs: &HashMap<String, String>, previous: &str) -> String {
    let mut result = String::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        result.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            result.push_str(&rest[start..]);
            return result;
        };
        let key = after[..end].trim();
        if key == "prev" {
            result.push_str(previous);
        } else if let Some(value) = outputs.get(key) {
            result.push_str(value);
        }
        rest = &after[end + 2..];
    }
    result.push_str(rest);
    result
}

struct StepSink;

impl EventSink for StepSink {
    fn notice(&mut self, text: &str) {
        eprintln!("[workflow] {text}");
    }

    fn step(&mut self, step: usize) {
        eprintln!("[workflow] step {step}");
    }

    fn token(&mut self, text: &str) {
        print!("{text}");
        let _ = std::io::stdout().flush();
    }

    fn tool_call(&mut self, name: &str, _arguments: &str) {
        eprintln!("[workflow] tool {name}");
    }

    fn finish(&mut self) {
        println!();
    }
}

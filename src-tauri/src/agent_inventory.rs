use super::*;
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

const PAGE_SIZE: usize = 16;
const MAX_ENTRIES: usize = 100_000;
const MAX_DEPTH: usize = 32;

fn directory_guard(path: &Path) -> Result<File> {
    let handle = OpenOptions::new()
        .read(true)
        .share_mode(3)
        .custom_flags(0x0220_0000)
        .open(path)
        .map_err(fail)?;
    let metadata = handle.metadata().map_err(fail)?;
    if !metadata.is_dir() || metadata.file_attributes() & 0x400 != 0 {
        return Err("AgentDirectoryRedirected".into());
    }
    Ok(handle)
}

struct Usage {
    bytes: u64,
    remaining: usize,
    complete: bool,
    deadline: Instant,
}

impl Usage {
    fn count(&mut self, directory: &Path, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH || self.remaining == 0 || Instant::now() >= self.deadline {
            self.complete = false;
            return Ok(());
        }
        let _guard = directory_guard(directory)?;
        for entry in fs::read_dir(directory).map_err(fail)? {
            if self.remaining == 0 || Instant::now() >= self.deadline {
                self.complete = false;
                break;
            }
            self.remaining -= 1;
            let Ok(entry) = entry else {
                self.complete = false;
                continue;
            };
            let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
                self.complete = false;
                continue;
            };
            if metadata.file_attributes() & 0x400 != 0 {
                self.complete = false;
            } else if metadata.is_dir() {
                if self.count(&entry.path(), depth + 1).is_err() {
                    self.complete = false;
                }
            } else if metadata.is_file() {
                self.bytes = self.bytes.saturating_add(metadata.len());
            }
        }
        Ok(())
    }
}

fn folders(path: &Path) -> Result<Vec<String>> {
    let mut names = Vec::new();
    for (index, entry) in fs::read_dir(path).map_err(fail)?.enumerate() {
        if index >= MAX_ENTRIES {
            return Err("AgentInventoryEntryLimit".into());
        }
        let entry = entry.map_err(fail)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(fail)?;
        if metadata.is_dir() && metadata.file_attributes() & 0x400 == 0 {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_owned());
            }
        }
    }
    names.sort_unstable();
    Ok(names)
}

pub(super) fn list(root: &Path, args: &[String]) -> Result<Value> {
    let after = match args {
        [] => ("", ""),
        [after] if after.len() <= 2048 => {
            let (project, name) = after.split_once('/').ok_or("InvalidAgentListArguments")?;
            if project.is_empty() || name.is_empty() || name.contains('/') {
                return Err("InvalidAgentListArguments".into());
            }
            (project, name)
        }
        _ => return Err("InvalidAgentListArguments".into()),
    };
    let instances = child_directory(root, "instances")?;
    let _instances = directory_guard(&instances)?;
    let agents = instances.join("Agent");
    match fs::symlink_metadata(&agents) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(json!({"schemaVersion":1,"items":[],"nextCursor":null}));
        }
        Err(error) => return Err(fail(error)),
        Ok(_) => (),
    }
    let _agents = directory_guard(&agents)?;
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut rows = Vec::new();
    let mut next = None;
    'projects: for project in folders(&agents)? {
        if project.as_str() < after.0 {
            continue;
        }
        let path = agents.join(&project);
        let _project = directory_guard(&path)?;
        for name in folders(&path)? {
            let key = format!("{project}/{name}");
            if (project.as_str(), name.as_str()) <= after {
                continue;
            }
            if rows.len() == PAGE_SIZE {
                next = rows.last().map(|item: &Value| item["id"].clone());
                break 'projects;
            }
            let mut usage = Usage {
                bytes: 0,
                remaining: MAX_ENTRIES,
                complete: true,
                deadline,
            };
            if usage.count(&path.join(&name), 0).is_err() {
                usage.complete = false;
            }
            rows.push(json!({"id":key,"project":project,"name":name,
                "relativePath":format!("instances/Agent/{key}"),"logicalBytes":usage.bytes,
                "sizeComplete":usage.complete}));
        }
    }
    Ok(json!({"schemaVersion":1,"items":rows,"nextCursor":next}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_unregistered_agent_files_without_reading_sibling_data() {
        let base = std::env::temp_dir().join(format!("ih-agent-list-{}", clock()));
        let instance = base.join("instances/Agent/Test/first/nested");
        let human = base.join("instances/Human/Test/first");
        fs::create_dir_all(&instance).unwrap();
        fs::create_dir_all(&human).unwrap();
        fs::write(instance.join("data"), b"test bytes").unwrap();
        fs::write(human.join("private"), b"private").unwrap();

        let canonical = fs::canonicalize(&base).unwrap();
        let output = list(&canonical, &[]).unwrap();
        assert_eq!(output["items"].as_array().unwrap().len(), 1);
        assert_eq!(output["items"][0]["id"], "Test/first");
        assert_eq!(output["items"][0]["logicalBytes"], 10);
        assert_eq!(output["items"][0]["sizeComplete"], true);
        assert!(output["nextCursor"].is_null());
        fs::remove_dir_all(&canonical).unwrap();
    }
}

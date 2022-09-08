use clap::{App, Arg};
use std::path::Path;

// #[tokio::main]
fn main() {
    let matches = App::new("cp-rs")
        .version("1.0")
        .author("Joshua Enokson <kilograhm@pm.me>")
        .about("Copies files in concurrency")
        .arg(
            Arg::new("src")
                .multiple(true)
                .required(true)
                // .index(1),
        )
        .arg(
            Arg::new("dest")
                .required(true)
                // .index(2),
        )
        .arg(
            Arg::new("verbose")
                .short('v')
                // .index(2),
        )
        .get_matches();

    let sources: Vec<&Path> = matches
        .values_of("src")
        .unwrap()
        .into_iter()
        .map(|value| Path::new(value))
        .collect();

    let dest = Path::new(matches.value_of("dest").unwrap());
    let use_verbose = matches.is_present("verbose");

    lib::main(&sources, &dest, use_verbose);
}

mod lib {
    use num_cpus;
    use std::fs;
    use std::io::{stdout, Write};
    use std::path::{Path, PathBuf};
    use std::process;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use log::{error, LevelFilter};
    use syslog::{BasicLogger, Facility, Formatter3164};

    #[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
    pub enum Entry {
        File(PathBuf, PathBuf), // source, dest
        Dir(PathBuf, PathBuf),  // source, dest
    }

    pub struct State {
        pub entries: Mutex<Vec<Entry>>,
    }

    pub fn send_to_error(msg: String) {
        error!("{}", msg);
    }

    pub fn read_dir<'a>(src: PathBuf, dir: &'a Path) -> Vec<Entry> {
        let mut entries = vec![];
        match fs::read_dir(dir) {
            Ok(read_dir) => {
                for entry_result in read_dir.into_iter() {
                    match entry_result {
                        Ok(entry) => match entry.file_type() {
                            Ok(file_type) => {
                                if file_type.is_dir() {
                                    entries.push(Entry::Dir(src.to_path_buf(), entry.path().to_path_buf()))
                                } else if file_type.is_file() {
                                    entries.push(Entry::File(src.to_path_buf(), entry.path()))
                                }
                            }
                            Err(error) => send_to_error(error.to_string()),
                        },
                        Err(error) => send_to_error(error.to_string()),
                    }
                }
            }
            Err(error) => send_to_error(error.to_string()),
        }
        entries.reverse();
        return entries;
    }

    fn get_dest<'a>(src: &'a Path, dest: &'a Path, file: &'a Path) -> PathBuf {
        let relative = file.strip_prefix(src).expect("Not a prefix");
        dest.join(relative)
    }

    pub fn mk_dir<'a>(src: &'a Path, dest: &'a Path, dir: &'a Path) {
        let new_dest = get_dest(src, dest, dir);
        if let Err(error) = fs::create_dir_all(new_dest) {
            send_to_error(error.to_string())
        }
    }

    pub fn cp_file<'a>(src: &'a Path, dest: &'a Path, file: &'a Path) {
        let new_dest = get_dest(src, dest, file);
        if let Err(error) = fs::copy(file, new_dest) {
            send_to_error(error.to_string())
        }
    }

    pub fn run_task<'a>(dest: PathBuf, state: Arc<State>, use_verbose: bool) -> thread::JoinHandle<()>{
        thread::spawn(move || {
            if use_verbose {
                loop {
                    let entry_options = { state.entries.lock().unwrap().pop() };
                    match entry_options {
                        Some(entry) => match entry {
                            Entry::Dir(src, dir) => {
                                {
                                    let mut stdout = stdout().lock();
                                    write!(&mut stdout, "{} => {}\n", src.to_string_lossy(), get_dest(&src, &*dest, &dir).to_str().unwrap()).unwrap();
                                }
                                mk_dir(&src, &dest, &dir);
                                let mut new_entries = read_dir(src, &dir);
                                {
                                    let mut entries = state.entries.lock().unwrap();
                                    entries.append(&mut new_entries);
                                }
                            }
                            Entry::File(src, file) => {
                                {
                                    let mut stdout = stdout().lock();
                                    write!(&mut stdout, "{} => {}\n", src.to_string_lossy(), get_dest(&src, &*dest, &file).to_str().unwrap()).unwrap();
                                }
                                cp_file(&src, &dest, &file);
                            }
                        },
                        None => { break; }
                    }
                }
            } else {
                loop {
                    let entry_options = { state.entries.lock().unwrap().pop() };
                    match entry_options {
                        Some(entry) => match entry {
                            Entry::Dir(src, dir) => {
                                mk_dir(&src, &dest, &dir);
                                let mut new_entries = read_dir(src, &dir);
                                {
                                    let mut entries = state.entries.lock().unwrap();
                                    entries.append(&mut new_entries);
                                }
                            }
                            Entry::File(src, file) => {
                                cp_file(&src, &dest, &file);
                            }
                        },
                        None => { break; }
                    }
                }
            }
            
        })
    }

    pub fn main<'a>(sources: &'a [ &'a Path ], dest: &'a Path, use_verbose: bool) {
        let formatter = Formatter3164 {
            facility: Facility::LOG_USER,
            hostname: None,
            process: "cp-rs".into(),
            pid: process::id(),
        };

        let logger = syslog::unix(formatter).expect("could not connect to syslog");
        log::set_boxed_logger(Box::new(BasicLogger::new(logger)))
            .map(|()| log::set_max_level(LevelFilter::Info))
            .unwrap();

        if sources.len() > 1 {
            if dest.is_file() {
                panic!("If there are multiple sources, the destination must be a directory.");
            }
        }

        let mut entries: Vec<Entry> = vec![];

        for entry in sources {
            if entry.is_dir() {
                let path_str = entry.to_str().expect("Could not get path_str");
                if path_str.ends_with("/") {
                    entries.push(Entry::Dir(entry.to_path_buf(), entry.to_path_buf()));
                } else {
                    entries.push(Entry::Dir(entry.parent().unwrap().to_path_buf(), entry.to_path_buf()));
                }
            } else if entry.is_file() {
                entries.push(Entry::File(entry.parent().unwrap().to_path_buf(), entry.to_path_buf()));
            } else {
                panic!("Entry found is neither a file or directory");
            }
        }

        let main_state: Arc<State> = Arc::new(State {
            entries: Mutex::new(entries)
        });

        let cpu_count = num_cpus::get() as u64;
        let mut handles = vec![];
        for _ in 0..cpu_count {
            handles.push(run_task(dest.to_path_buf(), main_state.clone(), use_verbose));
        }
        for thread in handles {
            thread.join().unwrap();
        }
    }
}

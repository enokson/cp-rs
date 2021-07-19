use clap::{Arg, App};
use std::{
    path::PathBuf, 
    str::FromStr,
};

fn main() {

  let matches = App::new("cp-rs")
    .version("1.0")
    .author("Joshua Enokson <kilograhm@pm.me>")
    .about("Copies files in concurency")
    .arg(Arg::new("src")
        .about("Sets the src dir to use")
        .required(true)
        .index(1))
    .arg(Arg::new("dest")
      .about("Sets the dest dir to use")
      .required(true)
      .index(2))
    .get_matches();

  let src = PathBuf::from_str(matches.value_of("src").unwrap()).unwrap();
  let dest = PathBuf::from_str(matches.value_of("dest").unwrap()).unwrap();
  // let src = PathBuf::from_str("/home/joshua/Dropbox").unwrap();
  // let dest = PathBuf::from_str("/tmp/Documents").unwrap();

  lib::main(src, dest);

}


mod lib {
  use crossterm::{execute, cursor, terminal, style};
  use std::{fs::remove_file, io::{Stdout, Write, stdout, BufWriter}, str::FromStr};
  use std::collections::HashMap;
  use std::sync::{Arc, Mutex};
  use std::fs;
  use std::path::PathBuf;
  use std::thread;
  use num_cpus;
  // use std::time::Duration;

  #[derive(Debug, PartialEq, Eq)]
  pub enum Task {
    Initalizing,
    Idle,
    Scanning(PathBuf),
    Coping(PathBuf)
  }
  impl Clone for Task {
    fn clone(&self) -> Task {
      match self {
        Task::Idle => Task::Idle,
        Task::Initalizing => Task::Initalizing,
        Task::Coping(file) => Task::Coping(file.to_path_buf()),
        Task::Scanning(dir) => Task::Scanning(dir.to_path_buf())
      }
    }
  }

  #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
  pub enum Entry {
    File(PathBuf),
    Dir(PathBuf)
  }

  pub struct Worker {
    id: u16,
    task: Task
  }

  pub struct State {
    pub src: Mutex<PathBuf>,
    pub dest: Mutex<PathBuf>,
    pub entries: Mutex<Vec<Entry>>,
    pub next_id: Mutex<u16>,
    pub workers: Mutex<HashMap<u16, Worker>>,
    pub stdout: Mutex<Stdout>,
    pub stderror: Mutex<fs::File>,
    pub entries_processed: Mutex<u64>
  }

  pub fn send_to_error(state: Arc<State>, error: String) {
    let mut file = state.stderror.lock().unwrap();
    let file_ref = file.by_ref();
    let mut writer = BufWriter::new(file_ref);
    writer.write(format!("{}\n", error.to_string()).as_bytes()).unwrap();
  }

  pub fn read_dir(dir: &PathBuf, state: Arc<State>) -> Vec<Entry> {
    let mut entries = vec![];
    match fs::read_dir(dir) {
      Ok(read_dir) => {
        for entry_result in read_dir.into_iter() {
          match entry_result {
            Ok(entry) => match entry.file_type() {
              Ok(file_type) => {
                if file_type.is_dir() {
                  entries.push(Entry::Dir(entry.path()))
                } else if file_type.is_file() {
                  entries.push(Entry::File(entry.path()))
                }
              },
              Err(error) => send_to_error(state.clone(), error.to_string())
            },
            Err(error) => send_to_error(state.clone(), error.to_string())
          }
        }
      },
      Err(error) => send_to_error(state.clone(), error.to_string())
    }
    entries.reverse();
    return entries
  }

  fn get_dest(src: &PathBuf, dest: &PathBuf, file: &PathBuf) -> PathBuf {
    let relative = file.strip_prefix(src).expect("Not a prefix");
    dest.join(relative)
  }

  pub fn mk_dir(src: &PathBuf, dest: &PathBuf, dir: &PathBuf, state: Arc<State>) {
    let new_dest = get_dest(src, dest, dir);
    if let Err(error) = fs::create_dir_all(new_dest) {
      send_to_error(state.clone(), error.to_string())
    }
  }

  pub fn cp_file(src: &PathBuf, dest: &PathBuf, file: &PathBuf, state: Arc<State>) {
    let new_dest = get_dest(src, dest, file);
    if let Err(error) = fs::copy(file, new_dest) {
      send_to_error(state.clone(), error.to_string())
    }
  }

  pub fn update_task(id: &u16, task: Task, padding: &u16, state: Arc<State>) {
    let text: String;
    match &task {
      Task::Coping(file) => {
        text = format!("Copying {}", file.display());
      },
      Task::Idle => {
        text = format!("Idle");
      },
      Task::Initalizing => {
        text = format!("Initializing");
      },
      Task::Scanning(dir) => {
        text = format!("Scanning {}", dir.display());
      }
    }
    let last_task = {
      let mut workers = state.workers.lock().unwrap();
      let mut worker = workers.get_mut(&id).unwrap();
      let last_task = worker.task.clone();
      worker.task = task.clone();
      last_task
    };
    if last_task != task {
      let mut stdout = state.stdout.lock().unwrap();
      execute!(
        stdout,
        cursor::MoveTo(0, id + padding),
        terminal::Clear(terminal::ClearType::UntilNewLine),
        style::Print(text)
      ).unwrap();
      stdout.flush().unwrap();
    }
  }

  pub fn update_totals(state: Arc<State>) {
    let entries_processed = {
      state.entries_processed.lock().unwrap()
    };
    let entry_count = {
      state.entries.lock().unwrap().len() as u64
    };
    let mut stdout = state.stdout.lock().unwrap();
    execute!(
      stdout,
      cursor::MoveTo(0,0),
      terminal::Clear(terminal::ClearType::UntilNewLine),
      style::Print(format!("Entries processed: {}", entries_processed)),
      cursor::MoveTo(0,1),
      terminal::Clear(terminal::ClearType::UntilNewLine),
      style::Print(format!("Entries remaining: {}", entry_count))
    ).unwrap();
    stdout.flush().unwrap();
  }

  pub fn main(src: PathBuf, dest: PathBuf) {

    let tmp_error = PathBuf::from_str("/tmp/cp-rs-errors").unwrap();
    if tmp_error.exists() {
        remove_file(&tmp_error).expect("Could not remove old error log file.");
      // fs::remove_dir(&tmp_error).unwrap()
    }
    
    let error_file = fs::File::create(tmp_error).expect("Could not locate /tmp/cp-rs-errors");

    let main_state = Arc::new(State {
      src: Mutex::new(src.to_path_buf()),
      dest: Mutex::new(dest.to_path_buf()),
      entries: Mutex::new(vec![]),
      next_id: Mutex::new(0),
      workers: Mutex::new(HashMap::new()),
      stdout: Mutex::new(stdout()),
      stderror: Mutex::new(error_file),
      entries_processed: Mutex::new(0)
    });

    const PADDING: u16 = 3;
    let cpu_count = num_cpus::get() as u64;
    
    {
      let mut first_batch = read_dir(&src, main_state.clone());
      // println!("vec: {:#?}", &first_batch);
      let mut entries = main_state.entries.lock().unwrap();
      entries.append(&mut first_batch);
    }

    let entry_count = {
      main_state.entries.lock().unwrap().len() as u64
    };
    {
      let mut stdout = main_state.stdout.lock().unwrap();
      execute!(
        stdout,
        terminal::EnterAlternateScreen,
        terminal::Clear(terminal::ClearType::All),
        cursor::MoveTo(0, 0),
        style::Print(format!("Entries remaining: {}", entry_count)),
        cursor::MoveTo(0, 1),
        style::Print(format!("Entries processed: {}", 0)),
        cursor::MoveTo(0, 2),
        style::Print(format!("Threads: {}", cpu_count))
      ).unwrap();
      stdout.flush().unwrap();
    }
     
    let handles = (0..cpu_count)
      .into_iter()
      .map(|_| {
        let state = main_state.clone();
        let id = {
          let mut next_id = state.next_id.lock().unwrap();
          let id = *next_id;
          *next_id += 1;
          id
        };
        let worker = Worker {
          id,
          task: Task::Initalizing
        };
        {
          let mut workers = state.workers.lock().unwrap();
          workers.insert(id, worker);
        }
        update_task(&id, Task::Initalizing, &PADDING, state.clone());
        let src = {
          state.src.lock().unwrap().to_path_buf()
        };
        let dest = {
          state.dest.lock().unwrap().to_path_buf()
        };
        thread::spawn(move || {
          loop {
            let entry_options = {
              state.entries.lock().unwrap().pop()
            };
            match entry_options {
              Some(entry) => match entry {
                Entry::Dir(dir) => {
                  update_task(&id, Task::Scanning(dir.to_path_buf()), &PADDING, state.clone());
                  // thread::sleep(Duration::from_secs(2));
                  mk_dir(&src, &dest, &dir, state.clone());
                  let mut new_entries = read_dir(&dir, state.clone());
                  {
                    let mut entries = state.entries.lock().unwrap();
                    entries.append(&mut new_entries);
                  }
                  { 
                    let mut dirs_processed = state.entries_processed.lock().unwrap();
                    *dirs_processed += 1;
                  };
                  update_totals(state.clone())
                },
                Entry::File(file) => {
                  update_task(&id, Task::Coping(file.to_path_buf()), &PADDING, state.clone());
                  // thread::sleep(Duration::from_secs(2));
                  cp_file(&src, &dest, &file, state.clone());
                  { 
                    let mut files_processed = state.entries_processed.lock().unwrap();
                    *files_processed += 1;
                  };
                  update_totals(state.clone())
                }
              },
              None => { }
            }
            update_task(&id, Task::Idle, &PADDING, state.clone());
            let workers = state.workers.lock().unwrap();
            let mut should_break = true;
            for (_id, worker) in workers.iter() {
              match worker.task {
                Task::Idle => {

                },
                _ => {
                  should_break = false
                }
              }
            }
            if should_break {
              break;
            }
          }
        })
      })
      .collect::<Vec<thread::JoinHandle<_>>>();

      for thread in handles {
        thread.join().unwrap();
      }

      {
        let mut stdout = main_state.stdout.lock().unwrap();
        execute!(stdout, terminal::LeaveAlternateScreen).unwrap();
        stdout.flush().unwrap();
      }

  }

}

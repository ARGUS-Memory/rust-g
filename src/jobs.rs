//! Job system
use std::{
    cell::RefCell,
    collections::hash_map::{Entry, HashMap},
    sync::mpsc,
    thread,
};

struct Job {
    rx: mpsc::Receiver<Output>,
    handle: thread::JoinHandle<()>,
}

type Output = String;
type JobId = String;

const NO_RESULTS_YET: &str = "NO RESULTS YET";
const NO_SUCH_JOB: &str = "NO SUCH JOB";
const JOB_PANICKED: &str = "JOB PANICKED";

#[derive(Default)]
struct Jobs {
    map: HashMap<JobId, Job>,
    next_job: usize,
}

impl Jobs {
    fn start<F: FnOnce() -> Output + Send + 'static>(&mut self, f: F) -> JobId {
        let (tx, rx) = mpsc::channel();
        let handle = thread::spawn(move || {
            let _ = tx.send(f());
        });
        let id = self.next_job.to_string();
        self.next_job += 1;
        self.map.insert(id.clone(), Job { rx, handle });
        id
    }

    fn check(&mut self, id: &str) -> Output {
        let entry = match self.map.entry(id.to_owned()) {
            Entry::Occupied(occupied) => occupied,
            Entry::Vacant(_) => return NO_SUCH_JOB.to_owned(),
        };
        let result = match entry.get().rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Disconnected) => JOB_PANICKED.to_owned(),
            Err(mpsc::TryRecvError::Empty) => return NO_RESULTS_YET.to_owned(),
        };
        let _ = entry.remove().handle.join();
        result
    }
}

thread_local! {
    static JOBS: RefCell<Jobs> = RefCell::default();
}

pub fn start<F: FnOnce() -> Output + Send + 'static>(f: F) -> JobId {
    JOBS.with(|jobs| jobs.borrow_mut().start(f))
}

pub fn check(id: &str) -> String {
    JOBS.with(|jobs| jobs.borrow_mut().check(id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_and_check_immediate() {
        let id = start(|| "hello".to_owned());
        // Give the thread a moment to complete
        std::thread::sleep(std::time::Duration::from_millis(50));
        let result = check(&id);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_check_nonexistent_job() {
        let result = check("99999");
        assert_eq!(result, NO_SUCH_JOB);
    }

    #[test]
    fn test_check_completed_job_removed() {
        let id = start(|| "done".to_owned());
        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = check(&id); // consume result
        // Second check should say NO_SUCH_JOB since it was removed
        let result = check(&id);
        assert_eq!(result, NO_SUCH_JOB);
    }

    #[test]
    fn test_sequential_job_ids() {
        JOBS.with(|jobs| {
            let mut j = jobs.borrow_mut();
            let id1 = j.start(|| "a".to_owned());
            let id2 = j.start(|| "b".to_owned());
            let n1: usize = id1.parse().unwrap();
            let n2: usize = id2.parse().unwrap();
            assert_eq!(n2, n1 + 1);
        });
    }

    #[test]
    fn test_job_with_complex_result() {
        let id = start(|| {
            let mut s = String::new();
            for i in 0..100 {
                s.push_str(&i.to_string());
                s.push(',');
            }
            s
        });
        std::thread::sleep(std::time::Duration::from_millis(100));
        let result = check(&id);
        assert!(result.starts_with("0,1,2,"));
        assert!(result.ends_with(","));
    }
}

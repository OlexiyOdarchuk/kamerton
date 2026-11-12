//! Лише одна копія програми.
//!
//! Перша копія тримає блокування файлу `kamerton.lock`. Друга, побачивши блокування,
//! дописує свої посилання у `inbox.txt` і закривається, а перша підхоплює їх
//! і виходить на передній план. Так дві копії не качають ті самі інструменти
//! в ті самі файли й не прибирають одна одній тимчасові теки.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use eframe::egui;

pub struct Instance {
    lock: Option<File>,
    inbox: PathBuf,
}

pub enum Acquire {
    First(Instance),
    /// Вже працює інша копія; їй передано посилання з командного рядка.
    Second,
}

pub fn acquire(data: &Path) -> Acquire {
    let inbox = data.join("inbox.txt");
    let file = OpenOptions::new().create(true).truncate(false).write(true).open(data.join("kamerton.lock"));
    let Ok(file) = file else {
        // Не вдалося навіть відкрити файл блокування: працюємо як є.
        return Acquire::First(Instance { lock: None, inbox });
    };
    match file.try_lock() {
        Ok(()) => Acquire::First(Instance { lock: Some(file), inbox }),
        Err(TryLockError::WouldBlock) => {
            let args: Vec<String> = std::env::args().skip(1).filter(|a| !a.starts_with("--")).collect();
            if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&inbox) {
                let _ = writeln!(f, "{}", args.join(" "));
            }
            Acquire::Second
        }
        Err(TryLockError::Error(_)) => Acquire::First(Instance { lock: None, inbox }),
    }
}

impl Instance {
    /// Звільнити блокування, щоб нова копія (після оновлення) могла стартувати.
    pub fn release(&mut self) {
        self.lock = None;
    }

    /// Фоновий потік: кожне повідомлення від іншої копії (рядок з посиланнями) йде в `tx`.
    pub fn watch(&self, tx: Sender<String>, ctx: egui::Context) {
        let inbox = self.inbox.clone();
        let reading = inbox.with_extension("reading");
        let _ = std::fs::remove_file(&inbox);
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(500));
                // Перейменування атомарне: друга копія тим часом пише вже в новий файл.
                if std::fs::rename(&inbox, &reading).is_err() {
                    continue;
                }
                let text = std::fs::read_to_string(&reading).unwrap_or_default();
                let _ = std::fs::remove_file(&reading);
                for line in text.lines() {
                    if tx.send(line.to_string()).is_err() {
                        return;
                    }
                }
                ctx.request_repaint();
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn second_instance_is_detected() {
        let dir = std::env::temp_dir().join(format!("kamerton-lock-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = acquire(&dir);
        assert!(matches!(first, Acquire::First(Instance { lock: Some(_), .. })));
        assert!(matches!(acquire(&dir), Acquire::Second));
        drop(first);
        assert!(matches!(acquire(&dir), Acquire::First(_)));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

//! Asynchronous weather for the independent desktop card. Network I/O never blocks the UI.
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};
use laminar_sys::weather::Weather;

const RETRY: Duration = Duration::from_secs(60);
const REFRESH: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status { Loading, Ready, Unavailable, Stale }

pub struct Worker {
    lat: f32,
    lon: f32,
    receiver: Option<Receiver<Option<Weather>>>,
    next: Instant,
    pub data: Option<Weather>,
    pub status: Status,
}

impl Worker {
    pub fn new(lat: f32, lon: f32) -> Self {
        Self { lat, lon, receiver: None, next: Instant::now(), data: None, status: Status::Loading }
    }
    fn complete(&mut self, result: Option<Weather>) {
        if let Some(data) = result {
            self.data = Some(data);
            self.status = Status::Ready;
            self.next = Instant::now() + REFRESH;
        } else {
            self.status = if self.data.is_some() { Status::Stale } else { Status::Unavailable };
            self.next = Instant::now() + RETRY;
        }
    }
    pub fn poll(&mut self) {
        let result = match self.receiver.as_ref().map(Receiver::try_recv) {
            Some(Ok(data)) => Some(data),
            Some(Err(TryRecvError::Disconnected)) => Some(None),
            _ => None,
        };
        if let Some(result) = result {
            self.receiver = None;
            self.complete(result);
        }
        if self.receiver.is_none() && Instant::now() >= self.next {
            let (tx, rx) = mpsc::channel();
            let (lat, lon) = (self.lat, self.lon);
            if std::thread::Builder::new().name("desktop-weather".into()).spawn(move || {
                let _ = tx.send(laminar_sys::weather::fetch(lat, lon));
            }).is_ok() {
                self.receiver = Some(rx);
                if self.data.is_none() { self.status = Status::Loading; }
            } else {
                self.status = if self.data.is_some() { Status::Stale } else { Status::Unavailable };
                self.next = Instant::now() + RETRY;
            }
        }
    }
}

pub fn description(code: i32, russian: bool) -> &'static str {
    if !russian { return laminar_sys::weather::describe(code); }
    match code {
        0 => "Ясно", 1 => "Преимущественно ясно", 2 => "Переменная облачность", 3 => "Пасмурно",
        45 | 48 => "Туман", 51 | 53 | 55 => "Морось", 56 | 57 => "Ледяная морось",
        61 => "Небольшой дождь", 63 => "Дождь", 65 => "Сильный дождь", 66 | 67 => "Ледяной дождь",
        71 => "Небольшой снег", 73 => "Снег", 75 => "Сильный снег", 77 => "Снежные зёрна",
        80 | 81 => "Ливень", 82 => "Сильный ливень", 85 | 86 => "Снежный ливень",
        95 => "Гроза", 96 | 99 => "Гроза с градом", _ => "Нет описания",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample() -> Weather {
        Weather { temp_c: 7.0, code: 2, description: "Partly cloudy".into(), high_c: 9.0, low_c: 2.0,
            hourly: vec![], feels_c: 5.0, humidity: 50.0, wind_kmh: 2.0, uv: 0.0, is_day: true,
            sunrise: String::new(), sunset: String::new(), daily: vec![], now: String::new() }
    }
    #[test]
    fn failed_requests_do_not_fake_or_erase_weather() {
        let mut w = Worker::new(1.0, 2.0);
        w.complete(None);
        assert_eq!(w.status, Status::Unavailable);
        assert!(w.data.is_none());
        assert!(w.next > Instant::now());
        w.complete(Some(sample()));
        assert_eq!(w.status, Status::Ready);
        assert_eq!(w.data.as_ref().unwrap().temp_c, 7.0);
        w.complete(None);
        assert_eq!(w.status, Status::Stale);
        assert_eq!(w.data.as_ref().unwrap().temp_c, 7.0);
    }
    #[test]
    fn russian_descriptions_include_unknown() {
        assert_eq!(description(0, true), "Ясно");
        assert_eq!(description(95, true), "Гроза");
        assert_eq!(description(-1, true), "Нет описания");
        assert!(RETRY < REFRESH);
    }
}

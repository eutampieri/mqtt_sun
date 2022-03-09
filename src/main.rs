use chrono::{Datelike, Timelike};
use log::{info, LevelFilter};
use rumqttc::{Client, MqttOptions, QoS};
use simple_logger::SimpleLogger;
use syslog::{BasicLogger, Facility, Formatter3164};

#[derive(Debug, PartialEq)]
enum SunPosition {
    Night,
    AstronomicalDawn,
    NauticalDawn,
    CivilDawn,
    Sunrise,
    Sunset,
    CivilDusk,
    NauticalDusk,
    AstronomicalDusk,
    SolarNoon,
}

impl From<&SunPosition> for &'static str {
    fn from(s: &SunPosition) -> Self {
        match s {
            SunPosition::Night => "night",
            SunPosition::AstronomicalDawn => "astronomicalDawn",
            SunPosition::NauticalDawn => "nauticalDawn",
            SunPosition::CivilDawn => "civilDawn",
            SunPosition::Sunrise => "sunrise",
            SunPosition::Sunset => "sunset",
            SunPosition::CivilDusk => "civilDusk",
            SunPosition::NauticalDusk => "nauticalDusk",
            SunPosition::AstronomicalDusk => "astronomicalDusk",
            SunPosition::SolarNoon => "solarNoon",
        }
    }
}

impl From<(f64, bool)> for SunPosition {
    fn from(angle: (f64, bool)) -> Self {
        let is_morning = angle.1;
        let angle = angle.0.to_degrees() as i8;
        if is_morning {
            match angle {
                -18..=-13 => Self::AstronomicalDawn,
                -12..=-7 => Self::NauticalDawn,
                -6..=-1 => Self::CivilDawn,
                0..=90 => Self::Sunrise,
                _ => Self::Night,
            }
        } else {
            match angle {
                -18..=-13 => Self::AstronomicalDusk,
                -12..=-7 => Self::NauticalDusk,
                -6..=-1 => Self::CivilDusk,
                0..=90 => Self::Sunset,
                _ => Self::Night,
            }
        }
    }
}

fn get_mqtt_conn(server: &str) -> Client {
    let mut mqttoptions = MqttOptions::new(
        "rust_mqtt_sun",
        server,
        std::env::var("MQTT_PORT")
            .map(|x| x.parse().unwrap_or(1883))
            .unwrap_or(1883),
    );
    mqttoptions.set_keep_alive(5);

    let (client, mut connection) = Client::new(mqttoptions, 10);
    std::thread::spawn(move || for _ in connection.iter() {});
    client
}

fn init_logger() {
    if cfg!(debug_assertions) {
        SimpleLogger::new().init().unwrap();
    } else {
        let formatter = Formatter3164 {
            facility: Facility::LOG_SYSLOG,
            hostname: None,
            process: "sun_position".into(),
            pid: 0,
        };

        let logger = syslog::unix(formatter).expect("could not connect to syslog");
        log::set_boxed_logger(Box::new(BasicLogger::new(logger)))
            .map(|()| log::set_max_level(LevelFilter::Info))
            .expect("Couldn't setup logger!");
    }
}

fn publish_event(conn: &mut Client, event: &SunPosition, topic: &'static str) {
    let camel_case_sun_pos: &'static str = (event).into();
    conn.publish(
        topic,
        QoS::ExactlyOnce,
        false,
        camel_case_sun_pos.as_bytes(),
    )
    .unwrap_or_else(|_| log::error!("Could not publish event to MQTT server"));
}

fn date_to_julian(date: &chrono::Date<chrono::Local>) -> f64 {
    let today_greg = astro::time::Date {
        year: date.year() as i16,
        month: date.month() as u8,
        decimal_day: date.day() as f64,
        cal_type: astro::time::CalType::Gregorian,
    };
    astro::time::julian_day(&today_greg)
}

fn today_solar_noon(over: &astro::coords::GeographPoint) -> i64 {
    let today = chrono::Local::today();
    let today_jul = date_to_julian(&today);
    let today_jul_century = astro::time::julian_cent(today_jul);
    let sun_long =
        (280.46646 + today_jul_century * (36000.76983 + today_jul_century * 0.0003032)) % 360.0;
    let sun_anom = 357.52911 + today_jul_century * (35999.05029 - 0.0001537 * today_jul_century);
    let eccent_eart_orbit =
        0.016708634 - today_jul_century * (0.000042037 + 0.0000001267 * today_jul_century);
    let mean_obliq_ecliptic_corr = 23.0
        + (26.0
            + (21.448
                - today_jul_century
                    * (46.815 + today_jul_century * (0.00059 - today_jul_century * 0.001813)))
                / 60.0)
            / 60.0
        + 0.00256 * (125.04 - 1934.136 * today_jul_century).to_radians().cos();

    let var_y = (mean_obliq_ecliptic_corr / 2.0).to_radians().tan().powi(2);
    let eq_of_time = 4.0
        * (var_y * (2.0 * sun_long.to_radians()).sin()
            - 2.0 * eccent_eart_orbit * sun_anom.to_radians().sin()
            + 4.0
                * eccent_eart_orbit
                * var_y
                * sun_anom.to_radians().sin()
                * (2.0 * sun_long.to_radians()).cos()
            - 0.5 * var_y.powi(2) * (4.0 * sun_long.to_radians())
            - 1.25 * eccent_eart_orbit.powi(2) * (2.0 * sun_anom.to_radians()).sin())
        .to_degrees();

    let solar_noon_after = (720.0 - 4.0 * over.long - eq_of_time) / 1440.0;

    today
        .naive_local()
        .signed_duration_since(chrono::NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
        .num_seconds()
        + (solar_noon_after * 24.0 * 3600.0) as i64
}

fn main() -> ! {
    /*for i in 0..240i64 {
        let start = 1628546400000i64;
        println!(
            "Ore {}\t{:?} ({}°)",
            i / 10,
            SunPosition::from((
                sun::pos(start + (i * 360_000) as i64, 44.34, 11.69).altitude,
                i <= 120
            )),
            sun::pos(start + (i * 360_000) as i64, 44.34, 11.69)
                .altitude
                .to_degrees()
        )
    }*/
    init_logger();
    let my_coords = astro::coords::GeographPoint {
        long: std::env::var("LON")
            .expect("Missing longitude")
            .parse()
            .expect("Invalid longitude"),
        lat: std::env::var("LAT")
            .expect("Missing latitude")
            .parse()
            .expect("Invalid latitude"),
    };
    let mut conn =
        get_mqtt_conn(&std::env::var("MQTT_BROKER").expect("Please provide a MQTT broker"));
    let mut old_sun_pos = None;
    let mut time_of_noon = None;
    loop {
        if let Ok(t) = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            // Check for noon
            if let Some(time) = time_of_noon {
                let now = t.as_secs();
                if now > time as u64 {
                    publish_event(&mut conn, &SunPosition::SolarNoon, "sun");
                    time_of_noon = None;
                }
            }
            // Check for next event
            let is_morning = chrono::Local::now().hour() <= 12;
            let sun_info = sun::pos(t.as_millis() as i64, my_coords.lat, my_coords.long);
            conn.publish(
                "sun/info",
                QoS::ExactlyOnce,
                false,
                format!("{}", sun_info.altitude.to_degrees()).as_bytes(),
            )
            .unwrap_or_else(|_| log::error!("Could not publish event to MQTT server"));
            let sun_pos = SunPosition::from((sun_info.altitude, is_morning));
            if let Some(o_p) = &old_sun_pos {
                if o_p == &sun_pos {
                    std::thread::sleep(std::time::Duration::from_secs(60));
                    continue;
                }
            }
            info!("Reached {:?}", sun_pos);
            publish_event(&mut conn, &sun_pos, "sun");
            // Check if we should calculate noon time
            if sun_pos == SunPosition::Sunrise {
                time_of_noon = Some(today_solar_noon(&my_coords));
                let naive =
                    chrono::NaiveDateTime::from_timestamp_opt(time_of_noon.unwrap(), 0).unwrap();
                let utc = chrono::DateTime::<chrono::Utc>::from_utc(naive, chrono::Utc);

                info!("Today solar noon will occour at {}", utc);
            }
            old_sun_pos = Some(sun_pos);
        }
    }
}

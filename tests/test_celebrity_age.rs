extern crate regex;
use regex::Regex;
use chrono::{NaiveDate, Datelike};

// Mock the info! macro
macro_rules! info {
    ($($arg:tt)*) => {
        println!($($arg)*)
    };
}

fn parse_date(date_str: &str) -> Option<NaiveDate> {
    // Try various date formats
    let formats = [
        "%d %B %Y",       // 20 April 2023
        "%B %d, %Y",      // April 20, 2023
        "%Y-%m-%d",       // 2023-04-20
        "%B %Y",          // April 2023
        "%d %b %Y",       // 20 Apr 2023
        "%b %d, %Y",      // Apr 20, 2023
    ];
    
    for format in &formats {
        if let Ok(date) = NaiveDate::parse_from_str(date_str, format) {
            return Some(date);
        }
    }
    
    None
}

fn calculate_age(birth_date: NaiveDate, death_date: NaiveDate) -> u32 {
    let mut age = death_date.year() - birth_date.year();
    
    // Adjust age if birthday hasn't occurred yet in the death year
    if death_date.month() < birth_date.month() || 
       (death_date.month() == birth_date.month() && death_date.day() < birth_date.day()) {
        age -= 1;
    }
    
    age as u32
}

fn main() {
    // Test with Eddie Van Halen's dates
    let birth_date_str = "January 26, 1955";
    let death_date_str = "October 6, 2020";
    
    println!("Testing age calculation with:");
    println!("Birth date: {}", birth_date_str);
    println!("Death date: {}", death_date_str);
    
    if let Some(birth) = parse_date(birth_date_str) {
        if let Some(death) = parse_date(death_date_str) {
            let age = calculate_age(birth, death);
            println!("\nCalculated age at death: {} years", age);
            
            // Format the full message
            let message = format!("He died on {} at the age of {}.", death_date_str, age);
            println!("\nFormatted message: {}", message);
        } else {
            println!("Failed to parse death date");
        }
    } else {
        println!("Failed to parse birth date");
    }
}

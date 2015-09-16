/// WebSocket client used for testing against the Autobahn Test Suite

extern crate ws;
extern crate env_logger;

use std::rc::Rc;
use std::cell::RefCell;
use ws::{connect, CloseCode, Message, Result};

const AGENT: &'static str = "WS-RS";

fn main () {
    env_logger::init().unwrap();

    let total = get_case_count().unwrap();
    let mut case_id = 1;


    while case_id <= total {

        let case_url = format!("ws://127.0.0.1:9001/runCase?case={}&agent={}", case_id, AGENT);

        connect(case_url, |out| {
            move |msg| {
                out.send(msg)
            }
        }).unwrap();

        case_id += 1
    }

    update_reports().unwrap();
}

fn get_case_count() -> Result<usize> {

    // sadly we need to use a RefCell because rust doesn't know that only one handler will ever
    // modify the total, ah well
    let total = Rc::new(RefCell::new(0));

    try!(connect("ws://127.0.0.1:9001/getCaseCount", |out| {

        let my_total = total.clone();

        move |msg: Message| {

            let count = try!(msg.as_text());

            *my_total.borrow_mut() = count.parse::<usize>().unwrap();

            out.close(CloseCode::Normal)
        }

    }));

    let total = *total.borrow();
    Ok(total)
}

fn update_reports() -> Result<()> {
    let report_url = format!("ws://127.0.0.1:9001/updateReports?agent={}", AGENT);

    connect(report_url, |out| {
        move |_| {
            out.close(CloseCode::Normal)
        }
    })
}

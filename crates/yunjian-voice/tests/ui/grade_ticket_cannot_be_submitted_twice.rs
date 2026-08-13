use yunjian_recite::{FsrsGrade, Scheduler};
use yunjian_voice::session::GradeTicket;

fn misuse(ticket: GradeTicket, scheduler: &mut Scheduler) {
    let _ = ticket.submit(scheduler, FsrsGrade::Good);
    let _ = ticket.submit(scheduler, FsrsGrade::Easy);
}

fn main() {}

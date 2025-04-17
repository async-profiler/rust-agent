#[inline(never)]
#[allow(deprecated)]
fn accidentally_slow() {
    std::thread::sleep_ms(10);
    std::hint::black_box(0);
}

#[inline(never)]
fn accidentally_slow_2() {
    accidentally_slow();
    std::hint::black_box(0);
}

#[inline(never)]
fn short_sleep() {
    std::thread::sleep(std::time::Duration::from_micros(100));
    std::hint::black_box(0);
}

#[inline(never)]
fn short_sleep_2() {
    short_sleep();
    std::hint::black_box(0);
}

pub async fn run() {
    let mut ts: Vec<tokio::task::JoinHandle<()>> = vec![];
    for _ in 0..16 {
        ts.push(tokio::task::spawn(async move {
            loop {
                tokio::task::yield_now().await;
                // make sure most time is spent in `short_sleep_2`,
                // but `accidentally_slow_2` will cause long polls.
                if rand::random::<f64>() < 0.001 {
                    accidentally_slow_2();
                } else {
                    short_sleep_2();
                }
            }
        }));
    }
    for t in ts {
        t.await.ok();
    }
}

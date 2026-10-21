#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bucket {
    Ok200,
    Taken400,
    Forbidden403,
    Rate429,
    Pair,
    Cf,
    Other,
}

pub fn classify(status: &str, body: &str) -> Bucket {
    let b = body;
    let bl = b.to_ascii_lowercase();
    if bl.contains("error code: 1015")
        || (bl.contains("cloudflare") && bl.contains("1015"))
        || bl.contains("you are being rate limited")
        || (bl.contains("cf-ray") && bl.contains("<!doctype html"))
    {
        return Bucket::Cf;
    }
    if b.contains("\"code\": 10008") || b.contains("\"code\":10008") {
        return Bucket::Pair;
    }
    if b.contains("\"code\": 50020")
        || b.contains("invalid or taken")
        || b.contains("already been taken")
    {
        return Bucket::Taken400;
    }
    if b.contains("\"code\": 60003") || b.contains("\"code\": 60008") || b.contains("two.factor") {
        return Bucket::Forbidden403;
    }
    if b.contains("\"code\": 30016") {
        return Bucket::Rate429;
    }
    match status {
        "200" => Bucket::Ok200,
        "400" => Bucket::Taken400,
        "403" => {
            if b.trim().is_empty() {
                Bucket::Pair
            } else {
                Bucket::Forbidden403
            }
        }
        "429" => Bucket::Rate429,
        _ => Bucket::Other,
    }
}

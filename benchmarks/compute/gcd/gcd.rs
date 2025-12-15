// GCD Benchmark
// Compute GCD for many number pairs using Euclidean algorithm

fn gcd(a: i64, b: i64) -> i64 {
    let mut x = a;
    let mut y = b;
    while y != 0 {
        let temp = y;
        y = x % y;
        x = temp;
    }
    x
}

fn main() {
    let iterations = 10_000_000i64;
    let mut total: i64 = 0;
    
    // Deterministic pairs based on loop index
    for i in 1..=iterations {
        let a = (i * 17) % 10000 + 1;
        let b = (i * 31) % 10000 + 1;
        total += gcd(a, b);
    }
    
    println!("Sum of {} GCDs: {}", iterations, total);
}

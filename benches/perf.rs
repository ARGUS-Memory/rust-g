// rust-g performance baseline — run before and after optimizations to measure delta.
//
// cargo bench --features "allow_non_32bit,default,worleynoise"
//
// HTML reports: target/criterion/

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main, black_box};

// JSON data builders (no external deps)
fn json_flat_array(n: usize) -> String {
    let mut s = String::with_capacity(n * 6);
    s.push('[');
    for i in 0..n {
        if i > 0 { s.push(','); }
        s.push('"');
        s.push_str(&format!("item{i}"));
        s.push('"');
    }
    s.push(']');
    s
}

fn json_nested_object(depth: usize, width: usize) -> String {
    fn build(out: &mut String, depth: usize, width: usize) {
        out.push('{');
        for i in 0..width {
            if i > 0 { out.push(','); }
            out.push_str(&format!("\"k{i}\":"));
            if depth > 1 {
                build(out, depth - 1, width);
            } else {
                out.push_str(&format!("{i}"));
            }
        }
        out.push('}');
    }
    let mut s = String::new();
    build(&mut s, depth, width);
    s
}

// JSON benchmarks
#[cfg(feature = "json")]
fn bench_json(c: &mut Criterion) {
    use rust_g::json::{validate, reformat, get_path};

    let small  = json_flat_array(50);        //  ~350 B
    let medium = json_flat_array(1_000);     //  ~8 KB
    let large  = json_flat_array(20_000);    // ~170 KB

    let nested = json_nested_object(4, 5);   //  ~1 KB nested

    let mut g = c.benchmark_group("json/validate");
    for (label, data) in [("350B", &small), ("8KB", &medium), ("170KB", &large)] {
        let bytes = data.as_bytes();
        g.bench_with_input(BenchmarkId::from_parameter(label), bytes, |b, src| {
            b.iter(|| validate(black_box(src), 32))
        });
    }
    g.finish();

    let mut g = c.benchmark_group("json/minify");
    for (label, data) in [("350B", &small), ("8KB", &medium), ("170KB", &large)] {
        let bytes = data.as_bytes();
        g.bench_with_input(BenchmarkId::from_parameter(label), bytes, |b, src| {
            b.iter(|| reformat(black_box(src), true))
        });
    }
    g.finish();

    let mut g = c.benchmark_group("json/prettify");
    for (label, data) in [("350B", &small), ("8KB", &medium), ("170KB", &large)] {
        let bytes = data.as_bytes();
        g.bench_with_input(BenchmarkId::from_parameter(label), bytes, |b, src| {
            b.iter(|| reformat(black_box(src), false))
        });
    }
    g.finish();

    let nested_bytes = nested.as_bytes();
    c.bench_function("json/get_path", |b| {
        b.iter(|| get_path(black_box(nested_bytes), black_box("k0.k1.k2")))
    });
}

// Hash benchmarks
#[cfg(feature = "hash")]
fn bench_hash(c: &mut Criterion) {
    use rust_g::hash::string_hash;

    let s_1kb: String  = "abcdef0123456789".repeat(64);    //  1 KB
    let s_100kb: String = "abcdef0123456789".repeat(6400); // 100 KB

    let mut g = c.benchmark_group("hash");
    for algo in ["md5", "sha1", "sha256", "sha512", "xxh64"] {
        for (label, data) in [("1KB", &s_1kb), ("100KB", &s_100kb)] {
            g.bench_with_input(
                BenchmarkId::new(algo, label),
                &(algo, data.as_str()),
                |b, &(alg, s)| b.iter(|| string_hash(black_box(alg), black_box(s))),
            );
        }
    }
    g.finish();
}

// Noise benchmarks (Perlin, with HashMap caching)
#[cfg(feature = "noise")]
fn bench_noise(c: &mut Criterion) {
    use rust_g::noise::get_at_coordinates;

    let mut g = c.benchmark_group("noise");

    // Cache hit: same seed every call → HashMap lookup, no insert
    g.bench_function("single_call_cache_hit", |b| {
        b.iter(|| get_at_coordinates(black_box("42"), black_box("0.5"), black_box("0.7")))
    });

    // Cache growth: unique seed each call → HashMap grows
    g.bench_function("100_unique_seeds", |b| {
        b.iter(|| {
            for i in 0..100u32 {
                let _ = get_at_coordinates(&i.to_string(), "0.5", "0.7");
            }
        })
    });

    // 1000 coordinate samples with a warm cache
    g.bench_function("1000_coords_warm", |b| {
        // Pre-warm the seed
        let _ = get_at_coordinates("999", "0.0", "0.0");
        b.iter(|| {
            for i in 0..1000u32 {
                let x = (i % 100) as f32 * 0.1;
                let y = (i / 100) as f32 * 0.1;
                let _ = get_at_coordinates("999", &x.to_string(), &y.to_string());
            }
        })
    });

    g.finish();
}

// Worleynoise benchmarks — shows O(n²) scaling
#[cfg(feature = "worleynoise")]
fn bench_worleynoise(c: &mut Criterion) {
    use rust_g::worleynoise::{worley_noise, get_nth_smallest_dist};
    use std::collections::HashSet;

    // Full generation at increasing sizes (shows quadratic scaling)
    let mut g = c.benchmark_group("worleynoise/generate");
    g.sample_size(10); // generation is slow at large sizes
    for size in [20u32, 40, 80] {
        g.bench_with_input(
            BenchmarkId::from_parameter(format!("{}x{}", size, size)),
            &size,
            |b, &sz| {
                b.iter(|| {
                    worley_noise(
                        black_box("5"),
                        black_box("0.5"),
                        black_box("80"),
                        black_box(&sz.to_string()),
                        black_box("1"),
                        black_box("3"),
                    )
                })
            },
        );
    }
    g.finish();

    // Isolate get_nth_smallest_dist — the O(n) HashSet clone bottleneck
    let mut g = c.benchmark_group("worleynoise/get_nth_smallest_dist");
    for n in [10usize, 30, 100, 300] {
        // Use a grid layout — guaranteed unique, no hash collisions
        let set: HashSet<(i32, i32)> = (0..n as i32)
            .map(|i| (i % 20, i / 20))
            .collect();
        let centre = (10i32, 10i32);

        g.bench_with_input(
            BenchmarkId::from_parameter(format!("n={n}")),
            &(centre, set),
            |b, (ctr, s)| {
                // Two calls per pixel — nth=0 (closest) and nth=1 (second closest)
                b.iter(|| {
                    let d0 = get_nth_smallest_dist(black_box(*ctr), black_box(0), black_box(s));
                    let d1 = get_nth_smallest_dist(black_box(*ctr), black_box(1), black_box(s));
                    black_box((d0, d1))
                })
            },
        );
    }
    g.finish();
}

// Acreplace benchmarks
#[cfg(feature = "acreplace")]
fn bench_acreplace(c: &mut Criterion) {
    use aho_corasick::AhoCorasickBuilder;

    let patterns: Vec<String> = (0..50).map(|i| format!("token{i}")).collect();
    let replacements: Vec<String> = (0..50).map(|i| format!("REPL{i}")).collect();
    let ac = AhoCorasickBuilder::new().build(&patterns).unwrap();

    // 1KB text with ~10 matches
    let text_1kb = {
        let mut t = String::with_capacity(1024);
        for i in 0..60 {
            t.push_str(&format!("some text token{} more text ", i % 50));
        }
        t
    };

    // 100KB text
    let text_100kb = text_1kb.repeat(100);

    let mut g = c.benchmark_group("acreplace/replace");
    for (label, text) in [("1KB", &text_1kb), ("100KB", &text_100kb)] {
        g.bench_with_input(
            BenchmarkId::from_parameter(label),
            text,
            |b, t| b.iter(|| ac.replace_all(black_box(t), &replacements)),
        );
    }
    g.finish();
}

// Pathfinder benchmarks — JSON parse + node registration
#[cfg(feature = "pathfinder")]
fn bench_pathfinder(c: &mut Criterion) {
    use rust_g::argus_json::parse_value;

    // Build a grid graph as JSON (nodes connected to neighbours)
    fn grid_json(w: usize) -> String {
        let mut nodes = Vec::new();
        let id = |x: usize, y: usize| x * w + y;
        for x in 0..w {
            for y in 0..w {
                let mut conn = Vec::new();
                if x > 0     { conn.push(id(x-1, y)); }
                if x < w-1   { conn.push(id(x+1, y)); }
                if y > 0     { conn.push(id(x, y-1)); }
                if y < w-1   { conn.push(id(x, y+1)); }
                let conn_str = conn.iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(",");
                nodes.push(format!(
                    "{{\"unique_id\":{},\"x\":{},\"y\":{},\"z\":0,\"connected_nodes_id\":[{}]}}",
                    id(x,y), x, y, conn_str
                ));
            }
        }
        format!("[{}]", nodes.join(","))
    }

    let small_json  = grid_json(5);  // 25 nodes
    let medium_json = grid_json(10); // 100 nodes

    let mut g = c.benchmark_group("pathfinder/parse_json");
    for (label, json) in [("25_nodes", &small_json), ("100_nodes", &medium_json)] {
        let bytes = json.as_bytes();
        g.bench_with_input(BenchmarkId::from_parameter(label), bytes, |b, src| {
            b.iter(|| parse_value(black_box(src)))
        });
    }
    g.finish();
}

// Group registration
#[cfg(feature = "json")]
criterion_group!(json_benches, bench_json);
#[cfg(not(feature = "json"))]
criterion_group!(json_benches, dummy);

#[cfg(feature = "hash")]
criterion_group!(hash_benches, bench_hash);
#[cfg(not(feature = "hash"))]
criterion_group!(hash_benches, dummy);

#[cfg(feature = "noise")]
criterion_group!(noise_benches, bench_noise);
#[cfg(not(feature = "noise"))]
criterion_group!(noise_benches, dummy);

#[cfg(feature = "worleynoise")]
criterion_group!(worley_benches, bench_worleynoise);
#[cfg(not(feature = "worleynoise"))]
criterion_group!(worley_benches, dummy);

#[cfg(feature = "acreplace")]
criterion_group!(acreplace_benches, bench_acreplace);
#[cfg(not(feature = "acreplace"))]
criterion_group!(acreplace_benches, dummy);

#[cfg(feature = "pathfinder")]
criterion_group!(pathfinder_benches, bench_pathfinder);
#[cfg(not(feature = "pathfinder"))]
criterion_group!(pathfinder_benches, dummy);

// Fallback for disabled features
#[allow(dead_code)]
fn dummy(_c: &mut Criterion) {}

criterion_main!(
    json_benches,
    hash_benches,
    noise_benches,
    worley_benches,
    acreplace_benches,
    pathfinder_benches
);

extern crate time;
#[cfg(feature = "write_buffers")]
extern crate byteorder;

use std::f64::consts::PI;
use time::precise_time_ns;

#[cfg(feature = "write_buffers")]
use std::io::BufWriter;
#[cfg(feature = "write_buffers")]
use std::io::prelude::*;
#[cfg(feature = "write_buffers")]
use std::fs::File;
#[cfg(feature = "write_buffers")]
use std::path::Path;
#[cfg(feature = "write_buffers")]
use byteorder::{WriteBytesExt, LittleEndian};


/// Audio sample rate for the test set, used for realtime speed
/// calculation
const SAMPLE_RATE: f64 = 48000.0;
/// Will allow to test buffer sizes up to 4096
const BUFFER_LEN_TESTS: u32 = 13;
/// Total length of samples the filter benchmarks are ran on
const SAMPLE_COUNT: u64 = 524288;
/// Select how many IIR filters should be applied consecutively
/// on each buffer during the benchmark
const FILTER_COUNT: usize = 100;


/// Square wave generator
struct SquareWave {
    switch_samples: usize,
    status: bool,
    progress: usize,
}

impl SquareWave {
    /// Builds a new `SquareWave` initialized with the oscillator frequency
    fn new(frequency: f64) -> SquareWave {
        SquareWave {
            switch_samples: (SAMPLE_RATE / frequency / 2.0).round() as usize,
            status: false,
            progress: 0,
        }
    }

    fn reset(&mut self) {
        self.status = false;
        self.progress = 0;
    }
}


/// 2nd order biquad filter
#[derive(Copy)]
struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,

    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Clone for Biquad {
    fn clone(&self) -> Biquad {
        *self
    }
}

impl Biquad {
    fn new() -> Biquad {
        Biquad {
            b0: 0.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Calculate coefficients and initialize the `Biquad` struct following
    /// audio EQ CookBook peak eq from Robert Bristow-Johnson
    fn peak_eq(fs: f64, f0: f64, q: f64, db_gain: f64) -> Biquad {
        let a = 10.0_f64.powf(db_gain / 40.0);
        let omega = 2.0 * PI * f0 / fs;

        let alpha = omega.sin() / (2.0 * q);

        let a0 = 1.0 + alpha / a;

        let b0 = (1.0 + alpha * a) / a0;
        let b1 = (-2.0 * omega.cos()) / a0;
        let b2 = (1.0 - alpha * a) / a0;
        let a2 = (1.0 - alpha / a) / a0;

        Biquad {
            b0: b0,
            b1: b1,
            b2: b2,
            a1: b1,
            a2: a2,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Reset filter's state accumulators
    fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

/// Reset a list of `Biquad`
fn reset_biquads(biquads: &mut [Biquad]) {
    for biquad in biquads {
        biquad.reset();
    }
}


/// Generate a buffer as `Vec` of a defined size
fn get_buffer_vec(length: usize) -> Vec<f64> {
    let mut vec: Vec<f64> = Vec::new();
    vec.resize(length, 0.0);
    vec
}


macro_rules! create_fill_buffer_function {
    ($func:ident) => (
        /// Fills the provided buffer using `SquareWave` generator
        fn $func(buf: &mut [f64], sqw: &mut SquareWave) {
            for sample in buf {
                if sqw.progress == sqw.switch_samples {
                    sqw.progress = 0;
                    sqw.status = !sqw.status;
                }

                *sample = if sqw.status { 0.5 } else { -0.5 };
                sqw.progress += 1;
            }
        }
    )
}

#[cfg(feature = "write_buffers")]
/// Write buffers to disk in order to verify the algorithms's integrity
///
/// Build with `cargo build --release --features write_buffers` then
/// run `md5sum /tmp/vec-array-perf-*`
/// Each file should be identical as well as identical to the C++ demo's output
struct OutputPcmFile {
    writer: BufWriter<File>,
}

#[cfg(feature = "write_buffers")]
impl OutputPcmFile {
    /// Creates a new output file used for integrity verification purposes
    fn new(path_name: String) -> OutputPcmFile {
        let path = format!("/tmp/vec-array-perf-rust_{}", path_name);

        std::fs::remove_file(path.as_str()).ok();

        let path = Path::new(path.as_str());
        let file = File::create(&path).unwrap();
        let stream = BufWriter::new(file);

        OutputPcmFile { writer: stream }
    }

    /// Write the provided buffer to disk
    fn write_buffer(&mut self, buf: &[f64]) {
        for sample in buf {
            let mut wtr = vec![];
            wtr.write_f64::<LittleEndian>(*sample).unwrap();
            self.writer.write(wtr.as_slice()).unwrap();
        }
    }
}


/// Displays the benchmark timing results and a real-time performance estimate
fn print_elapsed(msg: &str, start: u64, filter_count: usize) {
    let elapsed = precise_time_ns() - start;
    let duration = elapsed as f64 / filter_count as f64 / SAMPLE_COUNT as f64;
    let realtime = 1.0 / duration / SAMPLE_RATE * 1e+9;
    println!("\t{}:\t{:.3} ns\t{:.0}x realtime", msg, duration, realtime);
}


macro_rules! create_iir_function {
    ($func:ident) => (
        /// Apply the supplied `Biquad` digital filter coefficients using a
        /// Direct Form 2 IIR digital filter on the provided buffer
        fn $func(buf: &mut [f64], bq: &mut Biquad) {
            for y in buf {
                let x = *y;
                *y = (bq.b0 * x) + (bq.b1 * bq.x1) +  (bq.b2 * bq.x2)
                     - (bq.a1 * bq.y1) - (bq.a2 * bq.y2);

                bq.x2 = bq.x1;
                bq.x1 = x;

                bq.y2 = bq.y1;
                bq.y1 = *y;
            }
        }
    )
}


// Create fill_buffer, iir,  unique fill_buffer_size and iir_size functions
//
// They will be used for vector, array slice and fixed-size arrays
// benchmarks.
//
// The reason to create functions for a unique size is:
//  On many platforms, if a function works on a &mut [f64] input parameter
// called with different array or array slice sizes, the resulting speed
// is close or identical to to the performance with vectors
//
// However, on other platforms performance is noticeably higher if the iir
// function is only called with a single size of array as input parameter

create_fill_buffer_function!(fill_buffer);
create_fill_buffer_function!(fill_buffer_8);
create_fill_buffer_function!(fill_buffer_16);
create_fill_buffer_function!(fill_buffer_32);
create_fill_buffer_function!(fill_buffer_64);
create_fill_buffer_function!(fill_buffer_128);
create_fill_buffer_function!(fill_buffer_256);
create_fill_buffer_function!(fill_buffer_512);
create_fill_buffer_function!(fill_buffer_1024);
create_fill_buffer_function!(fill_buffer_2048);
create_fill_buffer_function!(fill_buffer_4096);

create_iir_function!(iir);
create_iir_function!(iir_8);
create_iir_function!(iir_16);
create_iir_function!(iir_32);
create_iir_function!(iir_64);
create_iir_function!(iir_128);
create_iir_function!(iir_256);
create_iir_function!(iir_512);
create_iir_function!(iir_1024);
create_iir_function!(iir_2048);
create_iir_function!(iir_4096);


fn main() {
    println!("Rust Vector and Array performance comparison");

    let mut sqw = SquareWave::new(50.0);

    // Generate an array of biquads that will be applied
    // with the iir function later
    //
    // The biquads's gain is switched each time between positive  negative
    // in order to keep the input signal within thr -1.0/+1.0 range expected
    // If FILTER_COUNT is set to a multiple of 2, the output signal will remain
    // near identical to the input, beside the noise and distortion introduced
    // by 64-bit calculations
    let mut biquad_gain_positive = true;
    let mut biquads = [Biquad::new(); FILTER_COUNT];
    for i in 0..FILTER_COUNT {
        let db_gain = if biquad_gain_positive { 2.0 } else { -2.0 };
        biquad_gain_positive = !biquad_gain_positive;
        biquads[i] = Biquad::peak_eq(SAMPLE_RATE, 50.0, 0.3, db_gain);
    }

    /// Iterate over buffer sizes
    for i in 3..BUFFER_LEN_TESTS {
        let buffer_len = (2_u64).pow(i) as usize;
        let buffer_count = SAMPLE_COUNT / buffer_len as u64;

        println!("\nBuffer size: {} samples", buffer_len);

        // Scope to run the benchmarks for vectors
        {
            sqw.reset();
            reset_biquads(&mut biquads);

            #[cfg(feature = "write_buffers")]
            let mut output = OutputPcmFile::new(format!("sized_vector_{}", buffer_len));

            let mut buf = get_buffer_vec(buffer_len);
            let start = precise_time_ns();
            for _ in 0..buffer_count {
                fill_buffer(buf.as_mut_slice(), &mut sqw);
                for f in 0..FILTER_COUNT {
                    iir(buf.as_mut_slice(), &mut biquads[f]);
                }

                #[cfg(feature = "write_buffers")]
                output.write_buffer(buf.as_slice());
            }
            print_elapsed("sized vector", start, FILTER_COUNT);
        }

        // Scope to run the benchmarks for sliced arrays
        {
            sqw.reset();
            reset_biquads(&mut biquads);

            #[cfg(feature = "write_buffers")]
            let mut output = OutputPcmFile::new(format!("array_slice_{}", buffer_len));

            let mut buf = [0.0; 4096];
            let start = precise_time_ns();
            for _ in 0..buffer_count {
                fill_buffer(&mut buf[0..buffer_len], &mut sqw);
                for f in 0..FILTER_COUNT {
                    iir(&mut buf[..buffer_len], &mut biquads[f]);
                }

                #[cfg(feature = "write_buffers")]
                output.write_buffer(&buf[0..buffer_len]);
            }
            print_elapsed("array slice", start, FILTER_COUNT);
        }

        // Scope to run the benchmarks for unique fixed-sizes arrays
        {
            sqw.reset();
            reset_biquads(&mut biquads);

            match buffer_len {
                8 => {
                    #[cfg(feature = "write_buffers")]
                    let mut output = OutputPcmFile::new(format!("whole_array_{}", buffer_len));

                    let mut buf = [0.0; 8];
                    let start = precise_time_ns();
                    for _ in 0..buffer_count {
                        fill_buffer_8(&mut buf, &mut sqw);
                        for f in 0..FILTER_COUNT {
                            iir_8(&mut buf, &mut biquads[f]);
                        }

                        #[cfg(feature = "write_buffers")]
                        output.write_buffer(&buf);
                    }
                    print_elapsed("whole array", start, FILTER_COUNT);
                }

                16 => {
                    #[cfg(feature = "write_buffers")]
                    let mut output = OutputPcmFile::new(format!("whole_array_{}", buffer_len));

                    let mut buf = [0.0; 16];
                    let start = precise_time_ns();
                    for _ in 0..buffer_count {
                        fill_buffer_16(&mut buf, &mut sqw);
                        for f in 0..FILTER_COUNT {
                            iir_16(&mut buf, &mut biquads[f]);
                        }

                        #[cfg(feature = "write_buffers")]
                        output.write_buffer(&buf);
                    }
                    print_elapsed("whole array", start, FILTER_COUNT);
                }

                32 => {
                    #[cfg(feature = "write_buffers")]
                    let mut output = OutputPcmFile::new(format!("whole_array_{}", buffer_len));

                    let mut buf = [0.0; 32];
                    let start = precise_time_ns();
                    for _ in 0..buffer_count {
                        fill_buffer_32(&mut buf, &mut sqw);
                        for f in 0..FILTER_COUNT {
                            iir_32(&mut buf, &mut biquads[f]);
                        }

                        #[cfg(feature = "write_buffers")]
                        output.write_buffer(&buf);
                    }
                    print_elapsed("whole array", start, FILTER_COUNT);
                }

                64 => {
                    #[cfg(feature = "write_buffers")]
                    let mut output = OutputPcmFile::new(format!("whole_array_{}", buffer_len));

                    let mut buf = [0.0; 64];
                    let start = precise_time_ns();
                    for _ in 0..buffer_count {
                        fill_buffer_64(&mut buf, &mut sqw);
                        for f in 0..FILTER_COUNT {
                            iir_64(&mut buf, &mut biquads[f]);
                        }

                        #[cfg(feature = "write_buffers")]
                        output.write_buffer(&buf);
                    }
                    print_elapsed("whole array", start, FILTER_COUNT);
                }

                128 => {
                    #[cfg(feature = "write_buffers")]
                    let mut output = OutputPcmFile::new(format!("whole_array_{}", buffer_len));

                    let mut buf = [0.0; 128];
                    let start = precise_time_ns();
                    for _ in 0..buffer_count {
                        fill_buffer_128(&mut buf, &mut sqw);
                        for f in 0..FILTER_COUNT {
                            iir_128(&mut buf, &mut biquads[f]);
                        }

                        #[cfg(feature = "write_buffers")]
                        output.write_buffer(&buf);
                    }
                    print_elapsed("whole array", start, FILTER_COUNT);
                }

                256 => {
                    #[cfg(feature = "write_buffers")]
                    let mut output = OutputPcmFile::new(format!("whole_array_{}", buffer_len));

                    let mut buf = [0.0; 256];
                    let start = precise_time_ns();
                    for _ in 0..buffer_count {
                        fill_buffer_256(&mut buf, &mut sqw);
                        for f in 0..FILTER_COUNT {
                            iir_256(&mut buf, &mut biquads[f]);
                        }

                        #[cfg(feature = "write_buffers")]
                        output.write_buffer(&buf);
                    }
                    print_elapsed("whole array", start, FILTER_COUNT);
                }

                512 => {
                    #[cfg(feature = "write_buffers")]
                    let mut output = OutputPcmFile::new(format!("whole_array_{}", buffer_len));

                    let mut buf = [0.0; 512];
                    let start = precise_time_ns();
                    for _ in 0..buffer_count {
                        fill_buffer_512(&mut buf, &mut sqw);
                        for f in 0..FILTER_COUNT {
                            iir_512(&mut buf, &mut biquads[f]);
                        }

                        #[cfg(feature = "write_buffers")]
                        output.write_buffer(&buf);
                    }
                    print_elapsed("whole array", start, FILTER_COUNT);
                }

                1024 => {
                    #[cfg(feature = "write_buffers")]
                    let mut output = OutputPcmFile::new(format!("whole_array_{}", buffer_len));

                    let mut buf = [0.0; 1024];
                    let start = precise_time_ns();
                    for _ in 0..buffer_count {
                        fill_buffer_1024(&mut buf, &mut sqw);
                        for f in 0..FILTER_COUNT {
                            iir_1024(&mut buf, &mut biquads[f]);
                        }

                        #[cfg(feature = "write_buffers")]
                        output.write_buffer(&buf);
                    }
                    print_elapsed("whole array", start, FILTER_COUNT);
                }

                2048 => {
                    #[cfg(feature = "write_buffers")]
                    let mut output = OutputPcmFile::new(format!("whole_array_{}", buffer_len));

                    let mut buf = [0.0; 2048];
                    let start = precise_time_ns();
                    for _ in 0..buffer_count {
                        fill_buffer_2048(&mut buf, &mut sqw);
                        for f in 0..FILTER_COUNT {
                            iir_2048(&mut buf, &mut biquads[f]);
                        }

                        #[cfg(feature = "write_buffers")]
                        output.write_buffer(&buf);
                    }
                    print_elapsed("whole array", start, FILTER_COUNT);
                }

                4096 => {
                    #[cfg(feature = "write_buffers")]
                    let mut output = OutputPcmFile::new(format!("whole_array_{}", buffer_len));

                    let mut buf = [0.0; 4096];
                    let start = precise_time_ns();
                    for _ in 0..buffer_count {
                        fill_buffer_4096(&mut buf, &mut sqw);
                        for f in 0..FILTER_COUNT {
                            iir_4096(&mut buf, &mut biquads[f]);
                        }

                        #[cfg(feature = "write_buffers")]
                        output.write_buffer(&buf);
                    }
                    print_elapsed("whole array", start, FILTER_COUNT);
                }

                _ => {}
            }
        }
    }
}

// -*- mode: rust; -*-
//
// Copyright (c) 2019 Web 3 Foundation
//
// Authors:
// - Jeffrey Burdges <jeff@web3.foundation>

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_code)]
#![doc = include_str!("../README.md")]


use ark_std::{
    UniformRand,
    borrow::{Borrow,BorrowMut},
    io::{self, Read, Write}, // Result
};
use ark_serialize::{CanonicalSerialize};
use ark_ff::{Field};

use rand_core::{RngCore,CryptoRng};

pub use sha3::{Shake128, digest::Update};
use sha3::digest::{XofReader, ExtendableOutput};


#[cfg(test)]
mod tests;

#[cfg(debug_assertions)]
pub mod debug;

/// Trascript labels.
/// 
/// We prefer if labels are `&'static [u8]` but of course
/// users might require labels provided by another langauge.
pub trait AsLabel {
    fn as_label(&self) -> &[u8];
}
impl AsLabel for &'static [u8] {
    fn as_label(&self) -> &[u8] { &self[..] }
}
impl<const N: usize> AsLabel for &'static [u8; N] {
    fn as_label(&self) -> &[u8] { &self[..] }
}

/// Identify a byte slice as a label, which requires this not be
/// user controlled data.
/// 
/// We use `Borrow<[u8]>` so that `IsLabel<[u8; N]>`, `IsLabel<&[u8]>`,
/// `IsLabel<[u8]>`, etc. all work correctly.  `AsRef` would permit the
/// `IsLabel<str>`, which maybe non-cannonical and cause breakage.
#[derive(Clone,Debug)]
pub struct IsLabel<T>(pub T);
impl<T: Borrow<[u8]>> AsLabel for IsLabel<T> {
    fn as_label(&self) -> &[u8] { self.0.borrow() }
}


/// All types interpretable as `Transcript`s, including primarily
/// `impl BorrowMut<Traanscript>` types like `Transcript` and
/// `&mut Transcript`.
/// 
/// We permit `&[u8]` and `AsLabel<T>` here too, but caution that
/// `&[u8]` needs internal applicaiton domain seperation. 
pub trait IntoTranscript {
    type Taken: BorrowMut<Transcript>;
    fn into_transcript(self) -> Self::Taken;
}
impl<B: BorrowMut<Transcript>> IntoTranscript for B {
    type Taken = B;
    fn into_transcript(self) -> B { self }
}
impl<T: Borrow<[u8]>> IntoTranscript for IsLabel<T> {
    type Taken = Transcript;
    fn into_transcript(self) -> Transcript {
        Transcript::new(self)
    }
}
impl<'a> IntoTranscript for &'a [u8] {
    type Taken = Transcript;
    fn into_transcript(self) -> Transcript {
        let mut t = Transcript::default();
        t.write_bytes(self);
        t.seperate();
        t
    }
}
impl<'a, const N: usize> IntoTranscript for &'a [u8; N] {
    type Taken = Transcript;
    fn into_transcript(self) -> Transcript {
        let mut t = Transcript::default();
        t.write_bytes(self);
        t.seperate();
        t
    }
}

/// Shake128 transcript style hasher.
#[derive(Clone)]
pub struct Transcript {
    /// Length writen between `seperate()` calls.  Always less than 2^31.
    /// `None` means `write` was not yet invoked, so seperate() does nothing.
    /// We need this to distinguish zero length write calls.
    length: Option<u32>,
    /// Is this a witness transcript?
    #[cfg(feature = "debug-transcript")]
    debug_name: &'static str,
    /// Actual Shake128 hasher being written to.
    h: Shake128,
}

impl Default for Transcript {
    /// Create a fresh empty `Transcript`.
    fn default() -> Transcript {
        #[cfg(feature = "debug-transcript")]
        println!("Initial Shake128 transcript..");
        Transcript {
            length: None,
            #[cfg(feature = "debug-transcript")]
            debug_name: "",
            h: Shake128::default(),
        } 
    }
}

impl Update for Transcript {
    fn update(&mut self, bytes: &[u8]) {
        self.write_bytes(bytes);
    }
}

impl Write for Transcript {
    // Always succeed fully
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.write_bytes(bytes);
        Ok(bytes.len())
    }

    // Always succeed immediately
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }    
}


impl Transcript {
    /// Write basic unlabeled domain seperator into the hasher.
    /// 
    /// Implemented by writing in big endian the number of bytes
    /// written since the previous `seperate` call, aka I2OSP(len,4)
    /// from [rfc8017](https://www.rfc-editor.org/rfc/rfc8017.txt).
    /// 
    /// We do nothing unless `write_bytes` was called previously, aka
    /// after the previous `seperate` call.  Invoking `write_bytes(b"")`
    /// before `seperate` forces seperation, aka aligns multiple code
    /// paths with convergent hashing, but in which users supply zero
    /// length inputs.
    pub fn seperate(&mut self) {
        #[cfg(feature = "debug-transcript")]
        println!("Shake128 {}transcript seperator: {}",self.debug_name, self.length);
        if let Some(l) = self.length {
            self.h.update( & l.to_be_bytes() ); 
        }
        self.length = None;
    }

    /// Write bytes into the hasher, increasing doain separator counter.
    /// 
    /// We wrap each 2^31 bytes into a seperate domain, instead of
    /// producing an error.
    pub fn write_bytes(&mut self, mut bytes: &[u8]) {
        const HIGH: u32 = 0x80000000;
        loop {
            let length = self.length.get_or_insert(0);
            let l = ark_std::cmp::min( (HIGH - 1 - *length) as usize, bytes.len() );
            #[cfg(feature = "debug-transcript")]
            match ark_std::str::from_utf8(bytes) {
                Ok(s) => {
                    println!("Shake128 {}transcript write of {} bytes: b\"{}\"", self.debug_name, l, s);
                }
                Err(_) => {
                    println!("Shake128 {}transcript write of {} bytes out of {}", self.debug_name, l, bytes.len());
                }
            }
            self.h.update( &bytes[0..l] );
            bytes = &bytes[l..];
            if bytes.len() == 0 {
                *length += u32::try_from(l).unwrap();
                return;
            }
            *length |= HIGH;
            self.seperate();
        }
    }

    /*
    /// I2OSP(len,4) from [rfc8017](https://www.rfc-editor.org/rfc/rfc8017.txt)
    /// with our own domain seperation 
    fn append_u32(&mut self, v: u32) {
        self.seperate();
        self.write_bytes(&v.to_be_bytes());
        self.seperate();
    }
    */

    /// I2OSP(len,8) from [rfc8017](https://www.rfc-editor.org/rfc/rfc8017.txt)
    /// with our own domain seperation 
    pub fn append_u64(&mut self, v: u64) {
        self.seperate();
        self.write_bytes(&v.to_be_bytes());
        self.seperate();
    }

    /// Write into the hasher items seralizable by Arkworks.
    /// 
    /// We `ensure_seperated` from any previously supplied user data,
    /// so we therfore suggest `label` be called in between `append`
    /// and `write`s of possibly empty user data.
    /// See concerns on `ensure_seperated`.
    /// 
    /// We use uncompressed serialization here for performance. 
    pub fn append<O: CanonicalSerialize+?Sized>(&mut self, itm: &O) {
        self.seperate();
        itm.serialize_uncompressed(&mut *self)
        .expect("ArkTranscript should infaillibly flushed"); 
        self.seperate();
    }
    // In concrete terms, `t.append(itm);` yields `t.ensure_seperated(); itm.serialize_uncompressed(&t);`,
    // while `t.seperate(); t.append(itm);` yields `t.seperate(); itm.serialize_uncompressed(&t);`,
    // which differ if preceeded by a `t.write(user_data);` with empty `user_data`.

    /// Write into the hasher a slice of items seralizable by Arkworks,
    /// exactly like invoking `append` repeatedly.
    pub fn append_slice<O,B>(&mut self, itms: &[B])
    where O: CanonicalSerialize+?Sized, B: Borrow<O>, 
    {
        self.seperate();
        for itm in itms.iter() {
            itm.borrow()
            .serialize_uncompressed(&mut *self)
            .expect("ArkTranscript should infaillibly flushed");
            self.seperate();
        }
    }

    /// Write domain separation label into the hasher,
    /// after first ending the previous write phase.
    pub fn label(&mut self, label: impl AsLabel) {
        self.seperate();
        self.write_bytes(label.as_label());
        self.seperate();
    }

    /// Create a fresh `Transcript` with an initial domain label.
    /// 
    /// We implicitly have an initial zero length user data write
    /// preceeding this first label.
    pub fn new(label: impl AsLabel) -> Transcript {
        let mut t = Transcript::default();
        t.label(label);
        t
    }

    /// Switch from writing to reading
    fn raw_reader(self) -> Reader {
        #[cfg(feature = "debug-transcript")]
        println!("Shake128 {}transcript XoF reader",self.debug_name);
        Reader(self.h.clone().finalize_xof())
    }

    /// Create a challenge reader.
    /// 
    /// Invoking `self.label(label)` has the same effect upon `self`,
    /// but the reader returnned cannot be obtained by any combinataion of other methods.
    pub fn challenge(&mut self, label: impl AsLabel) -> Reader {
        #[cfg(feature = "debug-transcript")]
        println!("Shake128 {}transcript challenge",self.debug_name);
        self.seperate();
        self.write_bytes(label.as_label());
        let reader = self.clone().raw_reader();
        self.seperate();
        reader
    }

    /// Forks transcript to prepare a witness reader.
    /// 
    /// We `clone` the transcript and `label` this clone, but do not
    /// touch the original.  After forking, you should write any
    /// secret seeds into the transcript, and then invoke `witness`
    /// with system randomness.
    pub fn fork(&self, label: impl AsLabel) -> Transcript {
        let mut fork = self.clone();
        #[cfg(feature = "debug-transcript")]
        {
            fork.debug_name = "witness ";
            println!("Shake128 {}transcript forked", self.debug_name);
        }
        // Invoking label forces an extra `seperate` vs `challenge`
        fork.label(label);
        fork
    }
    // In fact, `clone` alone works fine instead here, assuming you
    // correctly supply secret seeds and system randomness.
 
    /// Set the `debug_name` if you're doing anything complex, using clone, etc.
    #[cfg(feature = "debug-transcript")]
    pub fn set_debug_name(&mut self, debug_name: &'static str) {
        self.debug_name = debug_name;
    }

    // #[cfg(not(feature = "debug-transcript"))]
    // pub fn set_debug_name(&mut self, debug_name: &'static str) {
    // }

    /// Create a witness reader from a forked transcript.
    /// 
    /// We expect `rng` to be system randomness when used in production,
    /// ala `&mut rng_core::OsRng` or maybe `&mut rand::thread_rng()`,
    /// as otherwise you incur risks from any weaknesses elsewhere.
    /// 
    /// You'll need a deterministic randomness for test vectors of ourse, 
    /// ala `&mut ark_transcript::debug::TestVectorFakeRng`.
    /// We suggest implementing this choice inside your secret key type,
    /// along side whatever supplies secret seeds.
    pub fn witness(mut self, rng: &mut (impl RngCore+CryptoRng)) -> Reader {
        self.seperate();
        let mut rand = [0u8; 32];
        rng.fill_bytes(&mut rand);
        self.write_bytes(&rand);
        self.raw_reader()
    }
}


/// Shake128 transcript style XoF reader, used for both 
/// Fiat-Shamir challenges and witnesses.
pub struct Reader(sha3::Shake128Reader);

impl Reader {
    /// Read bytes from the transcript into the buffer.
    pub fn read_bytes(&mut self, buf: &mut [u8]) {
        self.0.read(buf);
    }

    /// Read bytes from the transcript. Always succeed fully.
    pub fn read_byte_array<const N: usize>(&mut self) -> [u8; N] {
        let mut buf = [0u8; N];
        self.0.read(&mut buf);
        buf
    }

    /// Sample a `T` using `ark_std:::UniformRand`
    /// 
    /// Arkworks always does rejection sampling so far, so
    /// constant-time-ness depends the object being sampled.
    pub fn read_uniform<T: UniformRand>(&mut self) -> T {
        <T as UniformRand>::rand(self)
    }

    /// Sample a field element using reduction mod the order.
    pub fn read_reduce<F: Field>(&mut self) -> F {
        self.read_uniform() // TODO: Use reduction mod ...
    }
}

impl XofReader for Reader {
    fn read(&mut self, dest: &mut [u8]) {
        self.read_bytes(dest);
    }
}

/// Read bytes from the transcript. Always succeed fully.
impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.read_bytes(buf);
        Ok(buf.len())
    }

    fn read_exact(&mut self, buf: &mut [u8]) -> io::Result<()> {
        self.read_bytes(buf);
        Ok(())
    }
}

/// Read bytes from the transcript. Always succeed fully
impl RngCore for Reader {
    fn next_u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        self.read_bytes(&mut b);
        u32::from_le_bytes(b)
    }
    fn next_u64(&mut self) -> u64 {
        let mut b = [0u8; 8];
        self.read_bytes(&mut b);
        u64::from_le_bytes(b)
    }
    fn fill_bytes(&mut self, dest: &mut [u8]) {
        self.read_bytes(dest);
    }
    fn try_fill_bytes(&mut self, dest: &mut [u8]) -> Result<(), rand_core::Error> {
        self.fill_bytes(dest);
        Ok(())
    }
}
// impl<T: BorrowMut<Transcript>> CryptoRng for TranscriptIO<T> { }


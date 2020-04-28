/* Copyright (C) 2020 Open Information Security Foundation
 *
 * You can copy, redistribute or modify this Program under the terms of
 * the GNU General Public License version 2 as published by the Free
 * Software Foundation.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * version 2 along with this program; if not, write to the Free Software
 * Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA
 * 02110-1301, USA.
 */

use crate::log::*;
use nom::branch::alt;
use nom::bytes::complete::tag;
use nom::character::complete::{digit1, multispace0, multispace1};
use nom::combinator::{map_res, opt};
use nom::sequence::{separated_pair, tuple};
use nom::IResult;
use std::ffi::CStr;
use std::os::raw::c_char;

const ASN1_DEFAULT_MAX_FRAMES: u16 = 30;

/// Parse the asn1 keyword and return a pointer to a `DetectAsn1Data`
/// containing the parsed options, returns null on failure
///
/// # Safety
///
/// pointer must be free'd using `rs_detect_asn1_free`
#[no_mangle]
pub unsafe extern "C" fn rs_detect_asn1_parse(input: *const c_char) -> *mut DetectAsn1Data {
    if input.is_null() {
        return std::ptr::null_mut();
    }

    let arg = match CStr::from_ptr(input).to_str() {
        Ok(arg) => arg,
        _ => {
            return std::ptr::null_mut();
        }
    };

    match asn1_parse_rule(&arg) {
        Ok((_rest, data)) => {
            let mut data = data;

            // Get configuration value
            if let Some(max_frames) = crate::conf::conf_get("asn1-max-frames") {
                if let Ok(v) = max_frames.parse::<u16>() {
                    data.max_frames = v;
                } else {
                    SCLogDebug!("Could not parse asn1-max-frames: {}", max_frames);
                };
            }

            Box::into_raw(Box::new(data))
        }
        Err(_) => std::ptr::null_mut(),
    }
}

/// Free a `DetectAsn1Data` object allocated by Rust
///
/// # Safety
///
/// ptr must be a valid object obtained using `rs_detect_asn1_parse`
#[no_mangle]
pub unsafe extern "C" fn rs_detect_asn1_free(ptr: *mut DetectAsn1Data) {
    if ptr.is_null() {
        return;
    }
    drop(Box::from_raw(ptr));
}

/// Struct to hold parsed asn1 keyword options
#[derive(Debug, PartialEq)]
pub struct DetectAsn1Data {
    pub bitstring_overflow: bool,
    pub double_overflow: bool,
    pub oversize_length: Option<u32>,
    pub absolute_offset: Option<u32>,
    pub relative_offset: Option<i32>,
    pub max_frames: u16,
}

impl Default for DetectAsn1Data {
    fn default() -> DetectAsn1Data {
        DetectAsn1Data {
            bitstring_overflow: false,
            double_overflow: false,
            oversize_length: None,
            absolute_offset: None,
            relative_offset: None,
            max_frames: ASN1_DEFAULT_MAX_FRAMES,
        }
    }
}

fn parse_u32_number(input: &str) -> IResult<&str, u32> {
    map_res(digit1, |digits: &str| digits.parse::<u32>())(input)
}
fn parse_i32_number(input: &str) -> IResult<&str, i32> {
    let (rest, negate) = opt(tag("-"))(input)?;
    let (rest, d) = map_res(digit1, |s: &str| s.parse::<i32>())(rest)?;
    let n = if negate.is_some() { -1 } else { 1 };
    Ok((rest, d * n))
}

/// Parse asn1 keyword options
pub(super) fn asn1_parse_rule(input: &str) -> IResult<&str, DetectAsn1Data> {
    // If nothing to parse, return
    if input.is_empty() {
        return Err(nom::Err::Error(nom::error::make_error(
            input,
            nom::error::ErrorKind::Eof,
        )));
    }

    // Rule parsing functions
    fn bitstring_overflow(i: &str) -> IResult<&str, &str> {
        tag("bitstring_overflow")(i)
    }

    fn double_overflow(i: &str) -> IResult<&str, &str> {
        tag("double_overflow")(i)
    }

    fn oversize_length(i: &str) -> IResult<&str, (&str, u32)> {
        separated_pair(tag("oversize_length"), multispace1, parse_u32_number)(i)
    }

    fn absolute_offset(i: &str) -> IResult<&str, (&str, u32)> {
        separated_pair(tag("absolute_offset"), multispace1, parse_u32_number)(i)
    }

    fn relative_offset(i: &str) -> IResult<&str, (&str, i32)> {
        separated_pair(tag("relative_offset"), multispace1, parse_i32_number)(i)
    }

    let mut data = DetectAsn1Data::default();

    let mut rest = input;

    // Parse the input and set data
    while !rest.is_empty() {
        let (
            new_rest,
            (
                _,
                bitstring_overflow,
                double_overflow,
                oversize_length,
                absolute_offset,
                relative_offset,
                _,
            ),
        ) = tuple((
            opt(multispace0),
            opt(bitstring_overflow),
            opt(double_overflow),
            opt(oversize_length),
            opt(absolute_offset),
            opt(relative_offset),
            opt(alt((multispace1, tag(",")))),
        ))(rest)?;

        if bitstring_overflow.is_some() {
            data.bitstring_overflow = true;
        } else if double_overflow.is_some() {
            data.double_overflow = true;
        } else if let Some((_, v)) = oversize_length {
            data.oversize_length = Some(v);
        } else if let Some((_, v)) = absolute_offset {
            data.absolute_offset = Some(v);
        } else if let Some((_, v)) = relative_offset {
            data.relative_offset = Some(v);
        } else {
            return Err(nom::Err::Error(nom::error::make_error(
                rest,
                nom::error::ErrorKind::Verify,
            )));
        }

        rest = new_rest;
    }

    Ok((rest, data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_case::test_case;

    // Test oversize_length
    #[test_case("oversize_length 1024",
        DetectAsn1Data { oversize_length: Some(1024), ..Default::default()};
        "check that we parse oversize_length correctly")]
    #[test_case("oversize_length",
        DetectAsn1Data::default() => panics "Error((\"oversize_length\", Verify))";
        "check that we fail if the needed arg oversize_length is not given")]
    // Test absolute_offset
    #[test_case("absolute_offset 1024",
        DetectAsn1Data { absolute_offset: Some(1024), ..Default::default()};
        "check that we parse absolute_offset correctly")]
    #[test_case("absolute_offset",
        DetectAsn1Data::default() => panics "Error((\"absolute_offset\", Verify))";
        "check that we fail if the needed arg absolute_offset is not given")]
    // Test relative_offset
    #[test_case("relative_offset 1024",
        DetectAsn1Data { relative_offset: Some(1024), ..Default::default()};
        "check that we parse relative_offset correctly")]
    #[test_case("relative_offset",
        DetectAsn1Data::default() => panics "Error((\"relative_offset\", Verify))";
        "check that we fail if the needed arg relative_offset is not given")]
    // Test bitstring_overflow
    #[test_case("bitstring_overflow",
        DetectAsn1Data { bitstring_overflow: true, ..Default::default()};
        "check that we parse bitstring_overflow correctly")]
    // Test double_overflow
    #[test_case("double_overflow",
        DetectAsn1Data { double_overflow: true, ..Default::default()};
        "check that we parse double_overflow correctly")]
    // Test combination of params
    #[test_case("oversize_length 1024, relative_offset 10",
        DetectAsn1Data { oversize_length: Some(1024), relative_offset: Some(10),
            ..Default::default()};
        "check for combinations of keywords (comma seperated)")]
    #[test_case("oversize_length 1024 absolute_offset 10",
        DetectAsn1Data { oversize_length: Some(1024), absolute_offset: Some(10),
            ..Default::default()};
        "check for combinations of keywords (space seperated)")]
    #[test_case("oversize_length 1024 absolute_offset 10, bitstring_overflow",
        DetectAsn1Data { bitstring_overflow: true, oversize_length: Some(1024),
            absolute_offset: Some(10), ..Default::default()};
        "check for combinations of keywords (space/comma seperated)")]
    #[test_case(
        "double_overflow, oversize_length 1024 absolute_offset 10,\n bitstring_overflow",
        DetectAsn1Data { double_overflow: true, bitstring_overflow: true,
            oversize_length: Some(1024), absolute_offset: Some(10),
            ..Default::default()};
        "1. check for combinations of keywords (space/comma/newline seperated)")]
    #[test_case(
        "\n\t double_overflow, oversize_length 1024 relative_offset 10,\n bitstring_overflow",
        DetectAsn1Data { double_overflow: true, bitstring_overflow: true,
            oversize_length: Some(1024), relative_offset: Some(10),
            ..Default::default()};
        "2. check for combinations of keywords (space/comma/newline seperated)")]
    // Test empty
    #[test_case("",
        DetectAsn1Data::default() => panics "Error((\"\", Eof))";
        "test that we break with a empty string")]
    // Test invalid rules
    #[test_case("oversize_length 1024, some_other_param 360",
        DetectAsn1Data::default() => panics "Error((\" some_other_param 360\", Verify))";
        "test that we break on invalid options")]
    #[test_case("oversize_length 1024,,",
        DetectAsn1Data::default() => panics "Error((\",\", Verify))";
        "test that we break on invalid format (missing option)")]
    #[test_case("bitstring_overflowabsolute_offset",
        DetectAsn1Data::default() => panics "Error((\"absolute_offset\", Verify))";
        "test that we break on invalid format (missing seperator)")]
    fn test_asn1_parse_rule(input: &str, expected: DetectAsn1Data) {
        let (rest, res) = asn1_parse_rule(input).unwrap();

        assert_eq!(0, rest.len());
        assert_eq!(expected, res);
    }
}
//! Thin clients for the Google Cloud APIs satz talks to directly (REST over
//! `reqwest`, ADC bearer token). One module per API; the pure parts — page
//! merging, matching — are separate functions so they can be tested without
//! a network.

pub(crate) mod resourcemanager;

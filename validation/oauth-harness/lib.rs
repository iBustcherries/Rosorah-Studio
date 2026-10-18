//! Executes the actual application's OAuth protocol unit tests without linking its GUI/media stack.
#![allow(dead_code)]
use anyhow::{Context, Result, ensure};
use reqwest::blocking::{Client,RequestBuilder};
use serde_json::{Value,json as j};
use std::{sync::{mpsc,atomic::{AtomicBool,Ordering}},time::{Duration,Instant,SystemTime,UNIX_EPOCH}};
#[derive(PartialEq,Eq,Clone,Copy)] enum Platform { Twitch, YouTube, Kick }
struct ClientConfig {id:String,secret:String,broker_url:String}
struct Token {access:String,refresh:String,expiry:u64}
enum Event {Authorize(String,String)}
fn now()->u64 {SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64}
fn field<'a>(v:&'a Value,key:&str)->Result<&'a str>{v[key].as_str().filter(|s|!s.is_empty()).context("missing field")}
fn json(request:RequestBuilder)->Result<Value>{Ok(request.send()?.error_for_status()?.json()?)}
fn wait(cancel:&AtomicBool,d:Duration)->bool{std::thread::sleep(d);cancel.load(Ordering::Relaxed)}
// Broker request transport is exercised separately by auth-broker/test.mjs.
mod kick {use super::*;pub(super) fn broker(c:&ClientConfig)->Result<reqwest::Url>{Ok(reqwest::Url::parse(&c.broker_url)?)}}
#[path="../../crates/app/src/accounts/oauth.rs"] mod oauth;

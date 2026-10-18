//! Helix reward operations run only in the existing authenticated account worker.
use super::*;
#[derive(Clone,Debug)]
pub enum Request { List, Save{ id:String, body:Value }, Delete(String), Queue(String), Resolve{reward:String,redemption:String,fulfilled:bool} }
const ENDPOINT:&str="https://api.twitch.tv/helix/channel_points/custom_rewards";
fn request(client:&Client,config:&ClientConfig,token:&Token,method:reqwest::Method,url:&str)->RequestBuilder{client.request(method,url).bearer_auth(&token.access).header("Client-Id",&config.id)}
fn list(client:&Client,config:&ClientConfig,token:&Token,user:&str,manageable:bool)->Result<Value>{json(request(client,config,token,reqwest::Method::GET,ENDPOINT).query(&[("broadcaster_id",user),("only_manageable_rewards",if manageable{"true"}else{"false"})]))}
pub fn validate(body:&Value)->Result<()> {
 let title=body["title"].as_str().unwrap_or("");ensure!(!title.trim().is_empty()&&title.chars().count()<=45,"Reward title must contain 1–45 characters");
 ensure!(body["cost"].as_u64().is_some_and(|n|n>=1&&n<=i32::MAX as u64),"Point cost must be between 1 and 2147483647");
 ensure!(body["prompt"].as_str().unwrap_or("").chars().count()<=200,"Description cannot exceed 200 characters");
 let color=body["background_color"].as_str().unwrap_or("");ensure!(color.len()==7&&color.starts_with('#')&&color[1..].bytes().all(|b|b.is_ascii_hexdigit()),"Choose a valid reward color");
 for (flag,key) in [("is_max_per_stream_enabled","max_per_stream"),("is_max_per_user_per_stream_enabled","max_per_user_per_stream"),("is_global_cooldown_enabled","global_cooldown_seconds")]{if body[flag].as_bool()==Some(true){ensure!(body[key].as_u64().is_some_and(|n|n>0),"Enabled limits must be greater than zero");}}
 Ok(())
}
pub(super) fn run(op:Request,client:&Client,config:&ClientConfig,token:&Token,user:&str)->Result<Value>{
 use reqwest::Method as M;
 if matches!(op,Request::List){let all=list(client,config,token,user,false)?;let own=list(client,config,token,user,true)?;return Ok(j!({"kind":"list","rewards":all["data"],"manageable":own["data"]}));}
 let id=match &op{Request::Save{id,..}=>id.as_str(),Request::Delete(id)|Request::Queue(id)=>id,Request::Resolve{reward,..}=>reward,_=>""};
 if !id.is_empty(){let own=list(client,config,token,user,true)?;ensure!(own["data"].as_array().is_some_and(|v|v.iter().any(|r|r["id"].as_str()==Some(id))),"Twitch only lets Rosorah manage rewards created by this app. Use Twitch's dashboard for this reward.");}
 match op {
  Request::Save{id,body}=>{validate(&body)?;let mut req=request(client,config,token,if id.is_empty(){M::POST}else{M::PATCH},ENDPOINT).query(&[("broadcaster_id",user)]);if !id.is_empty(){req=req.query(&[("id",&id)]);}let saved=json(req.json(&body))?;Ok(j!({"kind":"saved","data":saved["data"]}))},
  Request::Delete(id)=>{json(request(client,config,token,M::DELETE,ENDPOINT).query(&[("broadcaster_id",user),("id",&id)]))?;Ok(j!({"kind":"deleted"}))},
  Request::Queue(id)=>{let v=json(request(client,config,token,M::GET,&format!("{ENDPOINT}/redemptions")).query(&[("broadcaster_id",user),("reward_id",&id),("status","UNFULFILLED"),("first","50"),("sort","OLDEST")]))?;Ok(j!({"kind":"queue","reward":id,"data":v["data"],"more":v["pagination"]["cursor"].as_str().is_some_and(|s|!s.is_empty())}))},
  Request::Resolve{reward,redemption,fulfilled}=>{json(request(client,config,token,M::PATCH,&format!("{ENDPOINT}/redemptions")).query(&[("broadcaster_id",user),("reward_id",&reward),("id",&redemption)]).json(&j!({"status":if fulfilled{"FULFILLED"}else{"CANCELED"}})))?;Ok(j!({"kind":"resolved","reward":reward}))},
  Request::List=>unreachable!()
 }
}
#[cfg(test)]mod tests{use super::*;#[test]fn invalid_rewards_never_send(){assert!(validate(&j!({"title":"x","cost":0})).is_err());assert!(validate(&j!({"title":"Thank you","cost":100,"prompt":"","background_color":"#AABBCC"})).is_ok());}}

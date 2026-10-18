use crate::accounts::{Account,rewards::Request};
use eframe::egui;
use serde_json::{Value,json};
#[derive(Default)]pub struct RewardsUi{pub open:bool,rows:Vec<Value>,owned:Vec<String>,draft:Option<Value>,id:String,notice:String,queue:Vec<Value>,queue_reward:String,more:bool,delete:Option<String>}
fn send(account:&mut Account,op:Request){account.request_rewards(op);}
impl RewardsUi {
 pub fn refresh(&mut self,a:&mut Account){send(a,Request::List);}
 pub fn ui(&mut self,ctx:&egui::Context,a:&mut Account)->Option<(String,String)>{
  if !self.open{return None;}let mut link=None;
  if let Some(result)=a.reward_result.take(){match result{Err(e)=>self.notice=e,Ok(v)=>{match v["kind"].as_str(){Some("list")=>{self.rows=v["rewards"].as_array().cloned().unwrap_or_default();self.owned=v["manageable"].as_array().into_iter().flatten().filter_map(|r|r["id"].as_str().map(str::to_string)).collect();self.notice="Rewards refreshed".into();},Some("queue")=>{self.queue=v["data"].as_array().cloned().unwrap_or_default();self.queue_reward=v["reward"].as_str().unwrap_or("").into();self.more=v["more"].as_bool().unwrap_or(false);},Some("resolved")=>{send(a,Request::Queue(self.queue_reward.clone()));},_=>{self.draft=None;self.delete=None;self.notice="Saved to Twitch".into();send(a,Request::List);}}}}}
  let mut open=true;
  egui::Window::new("Twitch channel-point rewards").open(&mut open).default_width(640.).anchor(egui::Align2::CENTER_CENTER,egui::Vec2::ZERO).show(ctx,|ui|{
   ui.label("Manage rewards and connect them to alerts. Changes are saved to Twitch only when you click Save.");
   if !a.connected{ui.label(&a.status);ui.label("Connect or approve Twitch in Settings > Accounts. Your existing rewards are unchanged.");return;}
   ui.small("Twitch only allows this app to edit rewards it created. Other rewards can still trigger automations.");
   ui.add_enabled_ui(!a.reward_busy,|ui|{
    ui.horizontal(|ui|{if ui.button("Grant rewards access…").clicked(){a.grant_rewards();}if ui.button("Refresh").clicked(){self.refresh(a);}if ui.button("+ New reward").clicked(){self.id.clear();self.draft=Some(json!({"title":"","prompt":"","cost":100,"background_color":"#8B70F0","is_enabled":true,"is_user_input_required":false,"is_max_per_stream_enabled":false,"max_per_stream":1,"is_max_per_user_per_stream_enabled":false,"max_per_user_per_stream":1,"is_global_cooldown_enabled":false,"global_cooldown_seconds":60,"should_redemptions_skip_request_queue":false}));}});
    egui::ScrollArea::vertical().max_height(530.).show(ui,|ui|{
    for row in self.rows.clone(){let id=row["id"].as_str().unwrap_or("").to_string();let title=row["title"].as_str().unwrap_or("Reward").to_string();let owned=self.owned.contains(&id);ui.group(|ui|{ui.strong(format!("{} · {} points",title,row["cost"]));ui.label(if row["is_paused"].as_bool()==Some(true){"Paused"}else if row["is_enabled"].as_bool()==Some(false){"Disabled"}else{"Enabled"});ui.horizontal_wrapped(|ui|{
     if ui.button("Link automation").clicked(){link=Some((id.clone(),title.clone()));}
     if ui.add_enabled(owned,egui::Button::new("Edit")).clicked(){self.id=id.clone();self.draft=Some(body(&row));}
     if ui.add_enabled(owned,egui::Button::new("Redemptions")).clicked(){send(a,Request::Queue(id.clone()));}
     if ui.add_enabled(owned,egui::Button::new("Delete…")).clicked(){self.delete=Some(id.clone());}
    });if !owned{ui.small("Managed by Twitch or another app");}});}
    if let Some(id)=self.delete.clone(){ui.separator();ui.colored_label(egui::Color32::YELLOW,"Delete this reward from Twitch? This cannot be undone.");ui.horizontal(|ui|{if ui.button("Delete reward").clicked(){send(a,Request::Delete(id));}if ui.button("Keep reward").clicked(){self.delete=None;}});}
    if let Some(d)=&mut self.draft{ui.separator();ui.heading(if self.id.is_empty(){"New reward"}else{"Edit reward"});for(key,label)in[("title","Name"),("prompt","Description")]{let mut s=d[key].as_str().unwrap_or("").to_string();ui.label(label);ui.text_edit_singleline(&mut s);d[key]=json!(s);}
     let mut cost=d["cost"].as_u64().unwrap_or(100);ui.add(egui::DragValue::new(&mut cost).range(1..=i32::MAX as u64).prefix("Points: "));d["cost"]=json!(cost);
     let mut color=d["background_color"].as_str().unwrap_or("#8B70F0").to_string();ui.horizontal(|ui|{ui.label("Color (#RRGGBB)");ui.text_edit_singleline(&mut color);});d["background_color"]=json!(color);
     for(key,label)in[("is_enabled","Enabled"),("is_user_input_required","Ask viewer for a message"),("should_redemptions_skip_request_queue","Skip manual fulfillment queue")]{let mut b=d[key].as_bool().unwrap_or(false);ui.checkbox(&mut b,label);d[key]=json!(b);}
     if !self.id.is_empty(){let mut b=d["is_paused"].as_bool().unwrap_or(false);ui.checkbox(&mut b,"Paused");d["is_paused"]=json!(b);}
     ui.collapsing("Limits & cooldown",|ui|{for(flag,key,label)in[("is_max_per_stream_enabled","max_per_stream","Limit per stream"),("is_max_per_user_per_stream_enabled","max_per_user_per_stream","Limit per viewer per stream"),("is_global_cooldown_enabled","global_cooldown_seconds","Cooldown in seconds")]{let mut b=d[flag].as_bool().unwrap_or(false);let mut n=d[key].as_u64().unwrap_or(1);ui.horizontal(|ui|{ui.checkbox(&mut b,label);ui.add_enabled(b,egui::DragValue::new(&mut n).range(1..=604800));});d[flag]=json!(b);d[key]=json!(n);}});
     if ui.button("Save to Twitch").clicked(){match crate::accounts::rewards::validate(d){Ok(())=>send(a,Request::Save{id:self.id.clone(),body:d.clone()}),Err(e)=>self.notice=e.to_string()}}
    }
    if !self.queue_reward.is_empty(){ui.separator();ui.heading("Pending redemptions");if self.queue.is_empty(){ui.label("No pending redemptions.");}for r in self.queue.clone(){ui.horizontal_wrapped(|ui|{ui.label(format!("{}: {}",r["user_name"].as_str().unwrap_or("Viewer"),r["user_input"].as_str().unwrap_or("")));for(fulfilled,label)in[(true,"Fulfill"),(false,"Cancel & refund")]{if ui.button(label).clicked(){send(a,Request::Resolve{reward:self.queue_reward.clone(),redemption:r["id"].as_str().unwrap_or("").into(),fulfilled});}}});}if self.more{ui.small("Showing the oldest 50. Resolve these and refresh for the next batch.");}if ui.button("Refresh pending").clicked(){send(a,Request::Queue(self.queue_reward.clone()));}}
    });
   });if a.reward_busy{ui.spinner();ui.label("Contacting Twitch…");}if !self.notice.is_empty(){ui.label(&self.notice);}
  });self.open=open;link
 }
}
fn body(row:&Value)->Value{let mut d=json!({});for key in ["title","prompt","cost","background_color","is_enabled","is_user_input_required","should_redemptions_skip_request_queue","is_paused"]{if !row[key].is_null(){d[key]=row[key].clone();}}for(parent,flag,key)in[("max_per_stream_setting","is_max_per_stream_enabled","max_per_stream"),("max_per_user_per_stream_setting","is_max_per_user_per_stream_enabled","max_per_user_per_stream"),("global_cooldown_setting","is_global_cooldown_enabled","global_cooldown_seconds")]{d[flag]=json!(row[parent]["is_enabled"].as_bool().unwrap_or(false));d[key]=json!(row[parent][key].as_u64().unwrap_or(1));}d}

//! `ChatServer` is an actor. It maintains list of connection client session.
//! And manages available rooms. Peers send messages to other peers in same
//! room through `ChatServer`.

use actix::prelude::*;
use rand::{rngs::ThreadRng, Rng};
use std::collections::{HashMap, HashSet};
use serde::{Deserialize, Serialize};
use bcrypt::{verify};

use {Crud,establish_connection};
use actions::community::*;

#[derive(EnumString,ToString,Debug)]
pub enum UserOperation {
  Login, Register, Logout, CreateCommunity, Join, Edit, Reply, Vote, Delete, NextPage, Sticky
}


#[derive(EnumString,ToString,Debug)]
pub enum MessageToUser {
  Comments, Users, Ping, Pong, Error
}

#[derive(Serialize, Deserialize)]
pub struct ErrorMessage {
  op: String,
  error: String
}

/// Chat server sends this messages to session
#[derive(Message)]
pub struct WSMessage(pub String);

/// Message for chat server communications

/// New chat session is created
#[derive(Message)]
#[rtype(usize)]
pub struct Connect {
  pub addr: Recipient<WSMessage>,
}

/// Session is disconnected
#[derive(Message)]
pub struct Disconnect {
  pub id: usize,
}

/// Send message to specific room
#[derive(Message)]
pub struct ClientMessage {
  /// Id of the client session
  pub id: usize,
  /// Peer message
  pub msg: String,
  /// Room name
  pub room: String,
}

/// List of available rooms
pub struct ListRooms;

impl actix::Message for ListRooms {
  type Result = Vec<String>;
}

/// Join room, if room does not exists create new one.
#[derive(Message)]
pub struct Join {
  /// Client id
  pub id: usize,
  /// Room name
  pub name: String,
}

#[derive(Serialize, Deserialize)]
pub struct Login {
  pub username_or_email: String,
  pub password: String
}

impl actix::Message for Login {
  type Result = Result<LoginResponse, ErrorMessage>;
}

#[derive(Serialize, Deserialize)]
pub struct Register {
  username: String,
  email: Option<String>,
  password: String,
  password_verify: String
}

#[derive(Serialize, Deserialize)]
pub struct LoginResponse {
  op: String,
  jwt: String
}

impl actix::Message for Register {
  type Result = Result<LoginResponse, ErrorMessage>;
}

// #[derive(Serialize, Deserialize)]
// pub struct CreateCommunity {
//   name: String
// }

#[derive(Serialize, Deserialize)]
pub struct CreateCommunityResponse {
  op: String,
  community: Community
}

impl  actix::Message for CommunityForm {
  type Result = Result<CreateCommunityResponse, ErrorMessage>;
}

/// `ChatServer` manages chat rooms and responsible for coordinating chat
/// session. implementation is super primitive
pub struct ChatServer {
  sessions: HashMap<usize, Recipient<WSMessage>>, // A map from generated random ID to session addr
  rooms: HashMap<String, HashSet<usize>>, // A map from room name to set of connectionIDs
  rng: ThreadRng,
}

impl Default for ChatServer {
  fn default() -> ChatServer {
    // default room
    let mut rooms = HashMap::new();
    rooms.insert("Main".to_owned(), HashSet::new());

    ChatServer {
      sessions: HashMap::new(),
      rooms: rooms,
      rng: rand::thread_rng(),
    }
  }
}

impl ChatServer {
  /// Send message to all users in the room
  fn send_room_message(&self, room: &str, message: &str, skip_id: usize) {
    if let Some(sessions) = self.rooms.get(room) {
      for id in sessions {
        if *id != skip_id {
          if let Some(addr) = self.sessions.get(id) {
            let _ = addr.do_send(WSMessage(message.to_owned()));
          }
        }
      }
    }
  }
}

/// Make actor from `ChatServer`
impl Actor for ChatServer {
  /// We are going to use simple Context, we just need ability to communicate
  /// with other actors.
  type Context = Context<Self>;
}

/// Handler for Connect message.
///
/// Register new session and assign unique id to this session
impl Handler<Connect> for ChatServer {
  type Result = usize;

  fn handle(&mut self, msg: Connect, _: &mut Context<Self>) -> Self::Result {
    println!("Someone joined");

    // notify all users in same room
    self.send_room_message(&"Main".to_owned(), "Someone joined", 0);

    // register session with random id
    let id = self.rng.gen::<usize>();
    self.sessions.insert(id, msg.addr);

    // auto join session to Main room
    self.rooms.get_mut(&"Main".to_owned()).unwrap().insert(id);

    // send id back
    id
  }
}

/// Handler for Disconnect message.
impl Handler<Disconnect> for ChatServer {
  type Result = ();

  fn handle(&mut self, msg: Disconnect, _: &mut Context<Self>) {
    println!("Someone disconnected");

    let mut rooms: Vec<String> = Vec::new();

    // remove address
    if self.sessions.remove(&msg.id).is_some() {
      // remove session from all rooms
      for (name, sessions) in &mut self.rooms {
        if sessions.remove(&msg.id) {
          rooms.push(name.to_owned());
        }
      }
    }
    // send message to other users
    for room in rooms {
      self.send_room_message(&room, "Someone disconnected", 0);
    }
  }
}

/// Handler for Message message.
impl Handler<ClientMessage> for ChatServer {
  type Result = ();

  fn handle(&mut self, msg: ClientMessage, _: &mut Context<Self>) {
    self.send_room_message(&msg.room, msg.msg.as_str(), msg.id);
  }
}

/// Handler for `ListRooms` message.
impl Handler<ListRooms> for ChatServer {
  type Result = MessageResult<ListRooms>;

  fn handle(&mut self, _: ListRooms, _: &mut Context<Self>) -> Self::Result {
    let mut rooms = Vec::new();

    for key in self.rooms.keys() {
      rooms.push(key.to_owned())
    }

    MessageResult(rooms)
  }
}

/// Join room, send disconnect message to old room
/// send join message to new room
impl Handler<Join> for ChatServer {
  type Result = ();

  fn handle(&mut self, msg: Join, _: &mut Context<Self>) {
    let Join { id, name } = msg;
    let mut rooms = Vec::new();

    // remove session from all rooms
    for (n, sessions) in &mut self.rooms {
      if sessions.remove(&id) {
        rooms.push(n.to_owned());
      }
    }
    // send message to other users
    for room in rooms {
      self.send_room_message(&room, "Someone disconnected", 0);
    }

    if self.rooms.get_mut(&name).is_none() {
      self.rooms.insert(name.clone(), HashSet::new());
    }
    self.send_room_message(&name, "Someone connected", id);
    self.rooms.get_mut(&name).unwrap().insert(id);
  }

}

impl Handler<Login> for ChatServer {

  type Result = MessageResult<Login>;
  fn handle(&mut self, msg: Login, _: &mut Context<Self>) -> Self::Result {

    use actions::user::*;
    let conn = establish_connection();

    // Fetch that username / email
    let user: User_ = match User_::find_by_email_or_username(&conn, &msg.username_or_email) {
      Ok(user) => user,
      Err(e) => return MessageResult(
        Err(
          ErrorMessage {
            op: UserOperation::Login.to_string(), 
            error: "Couldn't find that username or email".to_string()
          }
          )
        )
    };

    // Verify the password
    let valid: bool = verify(&msg.password, &user.password_encrypted).unwrap_or(false);
    if !valid {
      return MessageResult(
        Err(
          ErrorMessage {
            op: UserOperation::Login.to_string(), 
            error: "Password incorrect".to_string()
          }
          )
        )
    }

    // Return the jwt
    MessageResult(
      Ok(
        LoginResponse {
          op: UserOperation::Login.to_string(), 
          jwt: user.jwt()
        }
        )
      )
  }
}

impl Handler<Register> for ChatServer {

  type Result = MessageResult<Register>;
  fn handle(&mut self, msg: Register, _: &mut Context<Self>) -> Self::Result {

    use actions::user::*;
    let conn = establish_connection();

    // Make sure passwords match
    if msg.password != msg.password_verify {
      return MessageResult(
        Err(
          ErrorMessage {
            op: UserOperation::Register.to_string(), 
            error: "Passwords do not match.".to_string()
          }
          )
        );
    }

    // Register the new user
    let user_form = UserForm {
      name: msg.username,
      email: msg.email,
      password_encrypted: msg.password,
      preferred_username: None,
      updated: None
    };

    // Create the user
    let inserted_user = match User_::create(&conn, &user_form) {
      Ok(user) => user,
      Err(e) => return MessageResult(
        Err(
          ErrorMessage {
            op: UserOperation::Register.to_string(), 
            error: "User already exists.".to_string() // overwrite the diesel error
          }
          )
        )
    };

    // Return the jwt
    MessageResult(
      Ok(
        LoginResponse {
          op: UserOperation::Register.to_string(), 
          jwt: inserted_user.jwt()
        }
        )
      )

  }
}


impl Handler<CommunityForm> for ChatServer {

  type Result = MessageResult<CommunityForm>;

  fn handle(&mut self, form: CommunityForm, _: &mut Context<Self>) -> Self::Result {
    let conn = establish_connection();
    let community = match Community::create(&conn, &form) {
      Ok(community) => community,
      Err(e) => return MessageResult(
        Err(
          ErrorMessage {
            op: UserOperation::CreateCommunity.to_string(), 
            error: "Community already exists.".to_string() // overwrite the diesel error
          }
          )
        )
    };
    
    MessageResult(
      Ok(
        CreateCommunityResponse {
          op: UserOperation::CreateCommunity.to_string(), 
          community: community
        }
        )
      )
  }
}
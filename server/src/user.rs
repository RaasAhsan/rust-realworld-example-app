extern crate bson;

extern crate iis;
extern crate hyper;

extern crate serde;
extern crate serde_json;

extern crate chrono;

extern crate crypto;

extern crate futures;
extern crate tokio_core;
extern crate tiberius;

extern crate toml;

extern crate reroute;

extern crate jwt;

extern crate futures_state_stream;

extern crate slug;

use futures::Future;
use tokio_core::reactor::Core;
use tiberius::{SqlConnection};
use tiberius::stmt::ResultStreamExt;
use tiberius::stmt::Statement;

use chrono::prelude::*;

use std::error::Error;
use std::fs::File;
use std::io::prelude::*;
use std::env;
use std::path::PathBuf;

use hyper::server::{Server, Request, Response};
use reroute::{RouterBuilder, Captures};
use hyper::header::{Authorization, Bearer};
use hyper::status::StatusCode;

use crypto::sha2::Sha256;

use jwt::{
    Header,
    Registered,
    Token,
};

use slug::slugify;

use super::*;

pub fn new_token(user_id: &str, _: &str) -> Option<String> {
    let header: jwt::Header = Default::default();
    let claims = jwt::Registered {
        iss: Some("mikkyang.com".into()),
        sub: Some(user_id.into()),
        ..Default::default()
    };
    let token = Token::new(header, claims);

    token.signed(b"secret_key", Sha256::new()).ok()
}

pub fn login(token: &str) -> Option<i32> {
    let token = Token::<Header, Registered>::parse(token).unwrap();

    if token.verify(b"secret_key", Sha256::new()) {
        match token.claims.sub {
            Some(token) => 
                match token.parse::<i32>() {
                    Ok(result) => Some(result),
                    Err(_) => None
                }
            ,_ => None
        }    
        
        
    } else {
        None
    }
}

fn get_user_from_row( row : tiberius::query::QueryRow ) -> (i32, String, Option<UserResult> ) {
    let email : &str = row.get(0);
    let token : &str = row.get(1);
    let user_name : &str = row.get(2);
    let bio : Option<&str> = row.get(3);
    let image : Option<&str> = row.get(4);
    let user_id : i32 = row.get(5);
    let result = Some(UserResult{user:User{ 
        email:email.to_string(), token:token.to_string(), bio:bio.map(|s| s.to_string()),
        image:image.map(|s| s.to_string()), username:user_name.to_string()
    }});
    (user_id, token.to_string(), result)
}

fn get_profile_from_row(row : tiberius::query::QueryRow) ->Option<ProfileResult> {
    let _ : &str = row.get(0);
    let _ : &str = row.get(1);
    let user_name : &str = row.get(2);
    let bio : Option<&str> = row.get(3);
    let image : Option<&str> = row.get(4);
    let f : i32 = row.get(5);
    let following : bool = f == 1;
    let result = Some(ProfileResult{profile:Profile{ 
        following:following, bio:bio.map(|s| s.to_string()),
        image:image.map(|s| s.to_string()), username:user_name.to_string()
    }});    
    result
}

pub fn registration_handler(mut req: Request, res: Response, _: Captures) {
    let mut body = String::new();
    let _ = req.read_to_string(&mut body);    
    let registration : Registration = serde_json::from_str(&body).unwrap();     

    let email : &str = &registration.user.email;
    let token : &str = &crypto::pbkdf2::pbkdf2_simple(&registration.user.password, 10000).unwrap();
    let username : &str = &registration.user.username;

    let mut result : Option<UserResult> = None; 
    {
        let mut lp = Core::new().unwrap();
        let future = SqlConnection::connect(lp.handle(), CONNECTION_STRING.as_str())
        .and_then(|conn| {
            conn.query( "
            INSERT INTO [dbo].[Users]
                ([Email]
                ,[Token]
                ,[UserName])
            VALUES
                (@P1
                ,@P2
                ,@P3); ; SELECT [Email],[Token],[UserName],[Bio],[Image], Id FROM [dbo].[Users] WHERE [Id] = SCOPE_IDENTITY()" , &[ &email, &token, &username]  )
            .for_each_row( |row| {
                                    let (_,_,result2) = get_user_from_row(row);
                                    result = result2;
                                    Ok(())
                }            
            )
        } );
        lp.run(future).unwrap();
    }

    if result.is_some() {
        let result = result.unwrap();
        let result = serde_json::to_string(&result).unwrap();
        let result : &[u8] = result.as_bytes();
        res.send(&result).unwrap();                        
    }        
}

pub fn update_user_handler(mut req: Request, res: Response, _: Captures) {
    let mut body = String::new();
    let _ = req.read_to_string(&mut body);    
    let token =  req.headers.get::<Authorization<Bearer>>(); 
    let mut result : Option<UserResult> = None; 
    match token {
        Some(token) => {
            let jwt = &token.0.token;
            let logged_in_user_id = login(&jwt);  

            match logged_in_user_id {
                Some(logged_in_user_id) => {
                    println!("logged_in_user {}, received '{}'", &logged_in_user_id, &body);

                    let update_user : UpdateUser = serde_json::from_str(&body).unwrap();     
                    let user_name : &str = &update_user.user.username.as_ref().map(|x| &**x).unwrap_or("");
                    let bio : &str = update_user.user.bio.as_ref().map(|x| &**x).unwrap_or("");
                    let image : &str = update_user.user.image.as_ref().map(|x| &**x).unwrap_or("");
                    let email : &str = &update_user.user.email.as_ref().map(|x| &**x).unwrap_or("");
                    let password : &str = &update_user.user.password.as_ref().map(|x| &**x).unwrap_or("");

                    let mut sql = Core::new().unwrap();
                    let update_user_cmd = SqlConnection::connect(sql.handle(), CONNECTION_STRING.as_str() )
                        .and_then(|conn| { conn.query(                            
                            "  UPDATE [dbo].[Users] SET 
                                [UserName]=CASE WHEN(LEN(@P2)=0) THEN UserName ELSE @P2 END,
                                [Bio]=CASE WHEN(LEN(@P3)=0) THEN Bio ELSE @P3 END,
                                [Image]=CASE WHEN(LEN(@P4)=0) THEN Image ELSE @P4 END,
                                [Email]=CASE WHEN(LEN(@P5)=0) THEN Email ELSE @P5 END
                                WHERE [Id] = @P1; 
                            SELECT [Email],[Token],[UserName],[Bio],[Image],Id FROM [dbo].[Users] WHERE [Id] = @P1
                            ", 
                            &[&logged_in_user_id, &user_name, &bio, &image, &email]
                            )
                            .for_each_row(|row| {
                                let (_,_,result2) = get_user_from_row(row);
                                result = result2;
                                Ok(())
                            })
                        }
                    );
                    sql.run(update_user_cmd).unwrap(); 
                },
                _ => {
                }
            }
        }
        _ => {

        }
    }
    if result.is_some() {
        let result = result.unwrap();
        let result = serde_json::to_string(&result).unwrap();
        let result : &[u8] = result.as_bytes();
        res.send(&result).unwrap();                        
    }      
}

pub fn get_current_user_handler(req: Request, res: Response, _: Captures) {
    let token = req.headers.get::<Authorization<Bearer>>(); 
    let mut result : Option<UserResult> = None; 
    match token {
        Some(token) => {
            let jwt = &token.0.token;
            let logged_in_user = login(&jwt);  

            match logged_in_user {
                Some(logged_in_user) => {
                    println!("logged_in_user {}", &logged_in_user);
                    let mut sql = Core::new().unwrap();
                    let get_user = SqlConnection::connect(sql.handle(), CONNECTION_STRING.as_str() )
                        .and_then(|conn| conn.query(                            
                            "SELECT [Email],[Token],[UserName],[Bio],[Image], Id FROM [dbo].[Users]
                                WHERE [Id] = @P1", &[&logged_in_user]
                        ).for_each_row(|row| {
                            let (_,_,result2) = get_user_from_row(row);
                            result = result2;
                            Ok(())
                        })
                    );
                    sql.run(get_user).unwrap(); 
                },
                _ => {
                }
            }
        }
        _ => {

        }
    }
    if result.is_some() {
        let result = result.unwrap();
        let result = serde_json::to_string(&result).unwrap();
        let result : &[u8] = result.as_bytes();
        res.send(&result).unwrap();                        
    }    
}

pub fn get_profile_handler(req: Request, res: Response, c: Captures) {
    let token = req.headers.get::<Authorization<Bearer>>(); 
    let logged_id : i32 =  
        match token {
            Some(token) => {
                let jwt = &token.0.token;
                login(&jwt).unwrap()

            }
            _ => 0
        };

    let caps = c.unwrap();
    let profile = &caps[0].replace("/api/profiles/", "");
    println!("profile: {}", profile);
    let mut result : Option<ProfileResult> = None; 

    {
        let mut sql = Core::new().unwrap();
        let get_profile_cmd = SqlConnection::connect(sql.handle(), CONNECTION_STRING.as_str() )
            .and_then(|conn| conn.query(                            
                "SELECT [Email],[Token],[UserName],[Bio],[Image] ,
( SELECT COUNT(*) FROM dbo.Followings F WHERE F.[FollowingId] = Id AND F.FollowerId = @P2 ) as Following
FROM [dbo].[Users]  WHERE [UserName] = @P1", &[&(profile.as_str()), &logged_id]
            ).for_each_row(|row| {
                result = get_profile_from_row(row);
                Ok(())
            })
        );
        sql.run(get_profile_cmd).unwrap(); 
    }

    if result.is_some() {
        let result = result.unwrap();
        let result = serde_json::to_string(&result).unwrap();
        let result : &[u8] = result.as_bytes();
        res.send(&result).unwrap();                        
    }   
}

pub fn unfollow_handler(req: Request, res: Response, c: Captures) {
    let token = req.headers.get::<Authorization<Bearer>>(); 
    let logged_id : i32 =  
        match token {
            Some(token) => {
                let jwt = &token.0.token;
                login(&jwt).unwrap()

            }
            _ => 0
        };

    let caps = c.unwrap();
    let profile = &caps[0].replace("/api/profiles/", "").replace("/follow", "");
    let mut result : Option<ProfileResult> = None; 

    {
        let mut sql = Core::new().unwrap();
        let delete_user = SqlConnection::connect(sql.handle(), CONNECTION_STRING.as_str() )
            .and_then(|conn| conn.query(                            
                "DELETE TOP (1) from [dbo].[Followings] WHERE [FollowerId] = @P2;
                SELECT TOP (1) [Email],[Token],[UserName],[Bio],[Image] ,
( SELECT COUNT(*) FROM dbo.Followings F WHERE F.[FollowingId] = Id AND F.FollowerId = @P2 ) as Following
FROM [dbo].[Users]  WHERE [UserName] = @P1", &[&(profile.as_str()), &logged_id]
            )
            .for_each_row(|row| {
                result = get_profile_from_row(row);
                Ok(())
            })
        );
        sql.run(delete_user).unwrap(); 
    }

    if result.is_some() {
        let result = result.unwrap();
        let result = serde_json::to_string(&result).unwrap();
        let result : &[u8] = result.as_bytes();
        res.send(&result).unwrap();                        
    }   
}

pub fn follow_handler(req: Request, res: Response, c: Captures) {
    let token = req.headers.get::<Authorization<Bearer>>(); 
    let logged_id : i32 =  
        match token {
            Some(token) => {
                let jwt = &token.0.token;
                login(&jwt).unwrap()

            }
            _ => 0
        };

    let caps = c.unwrap();
    let profile = &caps[0].replace("/api/profiles/", "").replace("/follow", "");
    println!("profile: {}", profile);
    let mut result : Option<ProfileResult> = None; 

    {
        let mut sql = Core::new().unwrap();
        let follow_cmd = SqlConnection::connect(sql.handle(), CONNECTION_STRING.as_str() )
            .and_then(|conn| conn.query(                            
                "INSERT INTO [dbo].[Followings] ([FollowingId] ,[FollowerId])
     SELECT @P2,(SELECT TOP (1) [Id]  FROM [Users] where UserName = @P1) EXCEPT SELECT [FollowingId] ,[FollowerId] from Followings;
                SELECT TOP 1 [Email],[Token],[UserName],[Bio],[Image] ,
( SELECT COUNT(*) FROM dbo.Followings F WHERE F.[FollowingId] = Id AND F.FollowerId = @P2 ) as Following
FROM [dbo].[Users]  WHERE [UserName] = @P1", &[&(profile.as_str()), &logged_id]
            ).for_each_row(|row| {
                result = get_profile_from_row(row);
                Ok(())
            })
        );
        sql.run(follow_cmd).unwrap(); 
    }

    if result.is_some() {
        let result = result.unwrap();
        let result = serde_json::to_string(&result).unwrap();
        let result : &[u8] = result.as_bytes();
        res.send(&result).unwrap();                        
    }   
}


pub fn authentication_handler(mut req: Request, mut res: Response, _: Captures) {
    let mut body = String::new();
    let _ = req.read_to_string(&mut body);    
    let login : Login = serde_json::from_str(&body).unwrap();    

    let mut result : Option<UserResult> = None; 
    {
        let mut sql = Core::new().unwrap();
        let email : &str = &login.user.email;
        let get_user_cmd = SqlConnection::connect(sql.handle(), CONNECTION_STRING.as_str() )
            .and_then(|conn| conn.query( "SELECT TOP 1 [Email],[Token],[UserName],[Bio],[Image], Id FROM [dbo].[Users] WHERE [Email] = @P1", &[&email] )
            .for_each_row(|row| {
                let (user_id,stored_hash,result2) = get_user_from_row(row);
                let authenticated_user = crypto::pbkdf2::pbkdf2_check( &login.user.password, &stored_hash);
                *res.status_mut() = StatusCode::Unauthorized;

                match authenticated_user {
                    Ok(valid) => {
                        if valid {                     
                            let token = new_token(user_id.to_string().as_ref(), &login.user.password).unwrap();

                            res.headers_mut().set(
                                Authorization(
                                    Bearer {
                                        token: token.to_owned()
                                    }
                                )
                            );
                            *res.status_mut() = StatusCode::Ok;
                            result = result2;
                        }
                    }
                    _ => { result = None; }
                }            
                Ok(())
            })
        );
        sql.run(get_user_cmd).unwrap(); 
    }

    if result.is_some() {
        let result = result.unwrap();
        let result = serde_json::to_string(&result).unwrap();
        let result : &[u8] = result.as_bytes();
        res.send(&result).unwrap();                        
    }  
}


#[cfg(test)]
use hyper::Client;
#[cfg(test)]
use user::rand::Rng;

#[cfg(test)]
pub static JACOB_PASSWORD : &'static str = r#"jakejake"#;

#[cfg(test)]
pub fn register_jacob() -> (std::string::String, std::string::String) {
    let client = Client::new();
    let since = since_the_epoch();

    let num = rand::thread_rng().gen_range(0, 1000);
    let user_name = format!( "Jacob-{}-{}", since, num );
    let email = format!( "jake-{}-{}@jake.jake", since, num );
    let body = format!(r#"{{"user":{{"username": "{}","email": "{}","password": "{}"}}}}"#, user_name, email, JACOB_PASSWORD); 

    let mut res = client.post("http://localhost:6767/api/users")
        .body(&body)
        .send()
        .unwrap();

    let mut buffer = String::new();
    res.read_to_string(&mut buffer).unwrap(); 

    let registration : UserResult = serde_json::from_str(&buffer).unwrap();   
    let registered_user = registration.user;  
    assert_eq!(registered_user.email, email); 
    assert_eq!(registered_user.username, user_name); 

    assert_eq!(res.status, hyper::Ok);  
    ( user_name, email )
}

#[cfg(test)]
pub fn login_jacob( email : std::string::String, password : String ) -> std::string::String {
    let client = Client::new();

    let body = format!(r#"{{"user":{{"email": "{}","password": "{}"}}}}"#, email, password);

    let mut res = client.post("http://localhost:6767/api/users/login")
        .body(&body)
        .send()
        .unwrap();
    let mut buffer = String::new();
    res.read_to_string(&mut buffer).unwrap(); 

    let login : UserResult = serde_json::from_str(&buffer).unwrap();   
    let logged_user = login.user;  
    assert_eq!(logged_user.email, email); 
        
    assert_eq!(res.status, hyper::Ok);
    let token = res.headers.get::<Authorization<Bearer>>().unwrap(); 
    let jwt = &token.0.token;
    jwt.to_owned()
}

#[cfg(test)]
fn follow_jacob() -> (std::string::String, std::string::String) {
    let client = Client::new();
    let ( user_name, email ) = register_jacob();
    let jwt = login_jacob( email, JACOB_PASSWORD.to_string() );
    let url = format!("http://localhost:6767/api/profiles/{}/follow", user_name);
    println!("url:{}", url);

    let mut res = client.post(&url)
        .header(Authorization(Bearer {token: jwt.to_owned()}))
        .send()
        .unwrap();

    let mut buffer = String::new();
    res.read_to_string(&mut buffer).unwrap(); 

    let profile_result : ProfileResult = serde_json::from_str(&buffer).unwrap();   
    let profile = profile_result.profile;  
    assert_eq!(profile.username, user_name); 
    assert_eq!(profile.following, true);

    assert_eq!(res.status, hyper::Ok);

    (user_name, jwt)
}

#[cfg(test)]
#[test]
fn registration_test() {
    register_jacob();
}

#[cfg(test)]
#[test]
fn login_test() {
    let client = Client::new();
    let ( user_name, email ) = register_jacob();
    login_jacob( email, JACOB_PASSWORD.to_string() );
}

#[cfg(test)]
#[test]
fn get_current_user_test() {
    let client = Client::new();
    let ( user_name, email ) = register_jacob();
    let jwt = login_jacob( email.to_owned(), JACOB_PASSWORD.to_string() );

    let url = format!("http://localhost:6767/api/user");

    let mut res = client.get(&url)
        .header(Authorization(Bearer {token: jwt}))
        .send()
        .unwrap();
    let mut buffer = String::new();
    res.read_to_string(&mut buffer).unwrap(); 

    let registration : UserResult = serde_json::from_str(&buffer).unwrap();   
    let registered_user = registration.user;  
    assert_eq!(registered_user.email, email); 
    assert_eq!(registered_user.username, user_name); 

    assert_eq!(res.status, hyper::Ok);
}

#[cfg(test)]
#[test]
fn update_user_test() {
    let client = Client::new();
    let ( user_name, email ) = register_jacob();
    let jwt = login_jacob( email.to_owned(), JACOB_PASSWORD.to_string() );

    let url = format!("http://localhost:6767/api/user");
    let new_user_name = user_name.to_owned() + "_CH";
    let body = format!(r#"{{"user": {{ "username":"{}"}}}}"#, new_user_name.to_owned());

    let mut res = client.put(&url)
        .header(Authorization(Bearer {token: jwt}))
        .body(&body)
        .send()
        .unwrap();
    let mut buffer = String::new();
    res.read_to_string(&mut buffer).unwrap(); 

    let registration : UserResult = serde_json::from_str(&buffer).unwrap();   
    let registered_user = registration.user;  
    assert_eq!(registered_user.email, email); 
    assert_eq!(registered_user.username, new_user_name); 

    assert_eq!(res.status, hyper::Ok);
}

#[cfg(test)]
#[test]
#[should_panic]
fn get_current_user_fail_test() {
    let client = Client::new();

    let url = format!("http://localhost:6767/api/user");

    let mut res = client.get(&url)
        .send()
        .unwrap();
    let mut buffer = String::new();
    res.read_to_string(&mut buffer).unwrap(); 

    let registration : UserResult = serde_json::from_str(&buffer).unwrap();   
    let registered_user = registration.user;  
    assert_eq!(res.status, hyper::Ok);
}


#[cfg(test)]
#[test]
#[should_panic]
fn login_fail_test() {
    let client = Client::new();
    let ( user_name, email ) = register_jacob();
    login_jacob( email, JACOB_PASSWORD.to_string() + "!" );
}

#[cfg(test)]
#[test]
fn profile_unlogged_test() {
    let client = Client::new();
    let ( user_name, email ) = register_jacob();
    let url = format!("http://localhost:6767/api/profiles/{}", user_name);

    let mut res = client.get(&url)
        .send()
        .unwrap();
    let mut buffer = String::new();
    res.read_to_string(&mut buffer).unwrap(); 

    let profile_result : ProfileResult = serde_json::from_str(&buffer).unwrap();   
    let profile = profile_result.profile;  
    assert_eq!(profile.username, user_name); 
    assert_eq!(profile.following, false); 

    assert_eq!(res.status, hyper::Ok);
}

#[cfg(test)]
#[test]
fn follow_test() {
    follow_jacob();
}


#[cfg(test)]
#[test]
fn profile_logged_test() {
    let client = Client::new();

    let ( user_name, email ) = register_jacob();
    let jwt = login_jacob( email, JACOB_PASSWORD.to_string() );
    let url = format!("http://localhost:6767/api/profiles/{}", user_name);

    let mut res = client.get(&url)
        .header(Authorization(Bearer {token: jwt}))
        .send()
        .unwrap();
    let mut buffer = String::new();
    res.read_to_string(&mut buffer).unwrap(); 
    
    let profile_result : ProfileResult = serde_json::from_str(&buffer).unwrap();   
    let profile = profile_result.profile;  
    assert_eq!(profile.username, user_name); 
    assert_eq!(profile.following, false);
    
    assert_eq!(res.status, hyper::Ok);
}

#[cfg(test)]
#[test]
fn unfollow_test() {
    let client = Client::new();

    let (user_name, jwt) = follow_jacob();
    let url = format!("http://localhost:6767/api/profiles/{}/follow", user_name);

    let mut res = client.delete(&url)
        .header(Authorization(Bearer {token: jwt}))
        .body("")
        .send()
        .unwrap();
    let mut buffer = String::new();
    res.read_to_string(&mut buffer).unwrap(); 

    let profile_result : ProfileResult = serde_json::from_str(&buffer).unwrap();   
    let profile = profile_result.profile;  
    assert_eq!(profile.username, user_name); 
    assert_eq!(profile.following, false);

    assert_eq!(res.status, hyper::Ok);
}

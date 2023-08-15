use std::str::FromStr;
use crate::*;

use actix_web::{
    http::header::{HeaderName, HeaderValue},
    body::BoxBody,
    HttpResponse
};

#[derive(Clone, Debug)]
pub struct CSRF {
    skip_urls: Vec<String>,
    salt: String,
    pub effective: chrono::Duration,
    pub header_name: HeaderName,
}

use sha2::Digest;

impl CSRF {
    pub fn token(salt: &str) -> String {
        let now = chrono::Utc::now().timestamp_millis().to_le_bytes();
        let mut src = vec![0; 8+salt.len()];
        src[0..8].copy_from_slice(&now);
        src[8..].copy_from_slice(salt.as_bytes());
        
        let hash = sha2::Sha256::digest(&src);
        let mut dst = vec![0; 8 + hash.len()];
        dst[0..8].copy_from_slice(&now);
        dst[8..].copy_from_slice(&hash);
        hex::encode(dst)
    }

    pub fn new(header_name: &str, skip_urls: Vec<String>, salt: &str, effective_duration: chrono::Duration) -> Self {
        CSRF {
            header_name: HeaderName::from_str(&header_name).unwrap(),
            skip_urls,
            salt: salt.to_string(),
            effective: effective_duration,
        }
    }

    pub fn generate_token(&self) -> String {
        CSRF::token(&self.salt)
    }

    pub fn verify_token(&self, test_token: &str) -> bool {
        let test_token = hex::decode(test_token);

        if let Ok(test_token) = test_token {
            if test_token.len() != 40 {
                return false;
            }
            
            let generate_time = test_token[0..8].try_into();
            if generate_time.is_err() {
                return false;
            }

            let generate_time = i64::from_le_bytes(generate_time.unwrap());

            let now = chrono::Utc::now().timestamp_millis();            
            if now < generate_time {
                return false
            }

            let delta = chrono::Duration::milliseconds(now - generate_time);
            if delta > self.effective {
                return false;
            }

            let mut hash = vec![0; 8 + self.salt.len()];
            hash[0..8].copy_from_slice(&test_token[0..8]);
            hash[8..].copy_from_slice(self.salt.as_bytes());

            let hash = sha2::Sha256::digest(&hash);
            return &hash[..] == &test_token[8..];
        }
        
        return false;
    }

    
}

impl Handler<BoxBody> for CSRF {
    fn skip(&self, req: &ServiceRequest) -> bool {
        let test_path = req.path();
        for url in &self.skip_urls {
            if match_uri(test_path, url) {
                return true;
            }
        }
        false
    }

    fn process(&self, req: ServiceRequest) -> Either<ServiceResponse, ServiceRequest> {
        match req.headers().get(&self.header_name) {
            Some(token) => {
                match token.to_str() {
                    Ok(token) => {
                        if self.verify_token(token) {
                            Either::Right(req)
                        } else {
                            Either::Left(req.into_response(HttpResponse::Forbidden().body("Forbidden")))
                        }
                        
                    },
                    Err(_) => {
                        Either::Left(req.into_response(HttpResponse::Forbidden().body("Forbidden")))
                    }
                }
            },
            None => {
                Either::Left(req.into_response(HttpResponse::Forbidden().body("Forbidden")))
            },
        }
    }

    fn post(&self, mut resp: ServiceResponse) -> ServiceResponse {
        if resp.status().is_success() {
            let token = self.generate_token();
            let token = HeaderValue::from_str(&token);
            if token.is_err() {
                return resp.into_response(HttpResponse::InternalServerError().body("InternalServerError"))
            }

            resp.headers_mut().insert(self.header_name.clone(), token.unwrap());
        }

        resp
    }
}


#[cfg(test)]
#[cfg(feature = "csrf")]
mod tests {
    
    use sha2::Digest;

    #[test]
    #[cfg(feature = "csrf")]
    fn test_hash() {
        let salt = "test".to_string();
        let now_src = chrono::Utc::now().timestamp_millis();
        println!("now src: {now_src}");
        let now = now_src.to_le_bytes();
        let mut src = Vec::with_capacity(8 + salt.len());
        src.extend_from_slice(&now);
        src.extend_from_slice(salt.as_bytes());
        assert_eq!(8 + salt.len(), src.len());
        assert_eq!(&now, &src[0..8]);
        assert_eq!(salt.as_bytes(), &src[8..]);

        let hash = sha2::Sha256::digest(&src);
        let mut dst = Vec::with_capacity(8 + hash.len());
        dst.extend_from_slice(&now);
        dst.extend_from_slice(&hash);
        let dst = hex::encode(dst);
        println!("{:?}, {}", dst, dst.len());

        

        let dst = hex::decode(dst).unwrap();
        let now_ans = i64::from_le_bytes(dst[0..8].try_into().unwrap());

        assert_eq!(now_src, now_ans);
        println!("now ans: {now_ans}");

        let mut hash = vec![0; 8 + salt.len()];
        hash[0..8].copy_from_slice(&dst[0..8]);
        hash[8..].copy_from_slice(salt.as_bytes());
        //hash.extend_from_slice(&dst[0..8]);
        //hash.extend_from_slice(salt.as_bytes());

        let hash = sha2::Sha256::digest(hash);
        println!("{:?}", &hash[..]);
        println!("{:?}", &dst[8..]);

        assert_eq!(&hash[..], &dst[8..]);

        if &hash[..] != &dst[8..] {
            println!("not ok");
        }

        //chrono::Duration::milliseconds(milliseconds)
        
    }

    #[test]
    #[cfg(feature = "csrf")]
    fn test_token() {
        let csrf = super::CSRF::new(
            "x-token",
            vec![],
            "cyberon",
            chrono::Duration::seconds(3600),
        );

        let token = csrf.generate_token();
        println!("{}", csrf.verify_token(&token));

    }
}
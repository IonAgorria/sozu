use nom::{
    bytes::complete::{is_not, tag, take_while},
    character::is_space,
    combinator::opt,
    multi::many0,
    sequence::terminated,
    IResult,
};

#[derive(Debug)]
pub struct RequestCookie<'a> {
    pub name: &'a [u8],
    pub value: &'a [u8],
    pub semicolon: Option<&'a [u8]>,
    pub spaces: &'a [u8],
}

impl<'a> RequestCookie<'a> {
    pub fn get_full_length(&self) -> usize {
        let semicolon = if self.semicolon.is_some() { 1 } else { 0 };
        let space = self.spaces.len();

        self.name.len() + self.value.len() + semicolon + space + 1 // +1 is for =
    }
}

pub fn is_cookie_value_char(chr: u8) -> bool {
    // chars: (space) , ;
    chr != 32 && chr != 44 && chr != 59
}

pub fn single_request_cookie(i: &[u8]) -> IResult<&[u8], RequestCookie> {
    let (i, name) = terminated(is_not("="), tag("="))(i)?;
    let (i, value) = take_while(is_cookie_value_char)(i)?;
    let (i, semicolon) = opt(tag(";"))(i)?;
    let (i, spaces) = take_while(is_space)(i)?;

    Ok((
        i,
        RequestCookie {
            name,
            value,
            semicolon,
            spaces,
        },
    ))
}

pub fn parse_request_cookies(input: &[u8]) -> Option<Vec<RequestCookie>> {
    let res: IResult<&[u8], Vec<RequestCookie>> = many0(single_request_cookie)(input);

    if let Ok((_, o)) = res {
        Some(o)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_cookies_request_cookies() {
        let none_cookies = parse_request_cookies(b"FOOBAR").unwrap();
        let some_cookies =
            parse_request_cookies(b"FOO=BAR;BAR=FOO; SOZUBALANCEID=0;   SOZU=SOZU").unwrap();

        assert_eq!(none_cookies.len(), 0);
        assert_eq!(some_cookies.len(), 4);

        assert_eq!(some_cookies[0].name, b"FOO");
        assert_eq!(some_cookies[0].value, b"BAR");
        assert_eq!(some_cookies[0].semicolon.is_some(), true);
        assert_eq!(some_cookies[0].spaces.len(), 0);

        assert_eq!(some_cookies[1].name, b"BAR");
        assert_eq!(some_cookies[1].value, b"FOO");
        assert_eq!(some_cookies[1].semicolon.is_some(), true);
        assert_eq!(some_cookies[1].spaces.len(), 1);

        assert_eq!(some_cookies[2].name, b"SOZUBALANCEID");
        assert_eq!(some_cookies[2].value, b"0");
        assert_eq!(some_cookies[2].semicolon.is_some(), true);
        assert_eq!(some_cookies[2].spaces.len(), 3);

        assert_eq!(some_cookies[3].name, b"SOZU");
        assert_eq!(some_cookies[3].value, b"SOZU");
        assert_eq!(some_cookies[3].semicolon.is_some(), false);
        assert_eq!(some_cookies[3].spaces.len(), 0);
    }
}

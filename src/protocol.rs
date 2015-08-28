use std::convert::{Into, From};

use self::OpCode::*;
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum OpCode {
    Continue,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
    Bad,
}

impl Into<u8> for OpCode {

    fn into(self) -> u8 {
        match self {
            Continue   =>   0,
            Text       =>   1,
            Binary     =>   2,
            Close      =>   8,
            Ping       =>   9,
            Pong       =>   10,
            Bad        => {
                debug_assert!(false, "Attempted to convert invalid opcode to u8. This is a bug.");
                8  // if this somehow happens, a close frame will help us tear down quickly
            }
        }
    }
}

impl From<u8> for OpCode {

    fn from(byte: u8) -> OpCode {
        match byte {
            0   =>   Continue,
            1   =>   Text,
            2   =>   Binary,
            8   =>   Close,
            9   =>   Ping,
            10  =>   Pong,
            _   =>   Bad
        }
    }
}

use self::CloseCode::*;
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum CloseCode {
    Normal,
    Away,
    Protocol,
    Unsupported,
    Status,
    Abnormal,
    Invalid,
    Policy,
    Size,
    Extension,
    Error,
    Restart,
    Again,
    Tls,
    Empty,
    Other(u16),
}

impl Into<u16> for CloseCode {

    fn into(self) -> u16 {
        let val: u16 = match self {
           Normal        =>   1000,
           Away          =>   1001,
           Protocol      =>   1002,
           Unsupported   =>   1003,
           Status        =>   1005,
           Abnormal      =>   1006,
           Invalid       =>   1007,
           Policy        =>   1008,
           Size          =>   1009,
           Extension     =>   1010,
           Error         =>   1011,
           Restart       =>   1012,
           Again         =>   1013,
           Tls           =>   1015,
           Empty         =>   0,
           Other(code)   =>   code,
        };
        val.to_be()
    }
}

impl From<u16> for CloseCode {

    fn from(mut be_u16: u16) -> CloseCode {
        be_u16 = u16::from_be(be_u16);
        match be_u16 {
            1000 => Normal,
            1001 => Away,
            1002 => Protocol,
            1003 => Unsupported,
            1005 => Status,
            1006 => Abnormal,
            1007 => Invalid,
            1008 => Policy,
            1009 => Size,
            1010 => Extension,
            1011 => Error,
            1012 => Restart,
            1013 => Again,
            1015 => Tls,
            0    => Empty,
            code => Other(code),
        }
    }
}


mod test {
    #![allow(unused_imports, unused_variables, dead_code)]
    use super::*;

    #[test]
    fn test_opcode_from_u8() {
        let byte = 2u8;
        assert_eq!(OpCode::from(byte), OpCode::Binary);
    }

    #[test]
    fn test_opcode_into_u8() {
        let text = OpCode::Text;
        let byte: u8 = text.into();
        assert_eq!(byte, 1u8);
    }

    #[test]
    fn test_closecode_from_u16() {
        let mut byte = 1008u16;
        byte = byte.to_be();
        assert_eq!(CloseCode::from(byte), CloseCode::Policy);
    }

    #[test]
    fn test_closecode_into_u16() {
        let text = CloseCode::Away;
        let byte: u16 = text.into();
        assert_eq!(byte, 1001u16.to_be());
    }
}

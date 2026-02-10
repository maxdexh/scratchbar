use std::{fmt, sync::Arc};

use serde::{Deserialize, Serialize};

use crate::tui::*;

#[derive(Debug, Serialize, Deserialize)]
pub(crate) enum ElemRepr {
    Print {
        raw: String,
        width: u16,
        height: u16,
    },
    MinSize {
        elem: Elem,
        width: u16,
        height: u16,
    },
    Image(ImageRepr),
    Stack(StackRepr),
    Block(BlockRepr),
    Interact(InteractRepr),
}
impl From<ElemRepr> for Elem {
    fn from(value: ElemRepr) -> Self {
        Self(Arc::new(value))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct BlockRepr {
    pub borders: BlockBorders,
    pub border_style: StyleRepr,
    pub border_set: BlockLineSet,
    pub inner: Option<Elem>,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct ImageRepr {
    pub img: RgbaImageWrap,
    pub sizing: ImageSizeMode,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct StackRepr {
    pub axis: Axis,
    pub items: Vec<StackItemRepr>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StackItemRepr {
    pub fill_weight: u16,
    pub elem: Elem,
}
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct InteractRepr {
    pub tag: InteractTag,
    pub normal: Elem,
    pub hovered: Option<Elem>,
}
#[derive(Default, Debug, Serialize, Deserialize)]
pub(crate) struct StyleRepr {
    pub begin: String,
    pub end: String,
}
impl From<TextStyle> for StyleRepr {
    fn from(value: TextStyle) -> Self {
        Self {
            begin: {
                let mut begin = String::new();
                value.begin(&mut begin).unwrap();
                begin
            },
            end: {
                let mut end = String::new();
                value.end(&mut end).unwrap();
                end
            },
        }
    }
}

pub(crate) struct RgbaImageWrap(pub image::RgbaImage);
const _: () = {
    impl std::ops::Deref for RgbaImageWrap {
        type Target = image::RgbaImage;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }
    impl fmt::Debug for ImageRepr {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.debug_struct("Image")
                .field("width", &self.img.width())
                .field("height", &self.img.height())
                .field("hash", &{
                    let mut hasher = std::hash::DefaultHasher::new();
                    std::hash::Hasher::write(&mut hasher, &self.img);
                    std::hash::Hasher::finish(&hasher)
                })
                .finish()
        }
    }
    use std::borrow::Cow;

    use serde::{Deserialize, Serialize};

    #[derive(Serialize, Deserialize)]
    struct RgbaImageDefer<'a> {
        width: u32,
        height: u32,
        buf: Cow<'a, [u8]>,
    }

    impl Serialize for RgbaImageWrap {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: serde::Serializer,
        {
            RgbaImageDefer {
                width: self.0.width(),
                height: self.0.height(),
                buf: Cow::Borrowed(self.0.as_raw()),
            }
            .serialize(serializer)
        }
    }
    impl<'de> Deserialize<'de> for RgbaImageWrap {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            let RgbaImageDefer { width, height, buf } = Deserialize::deserialize(deserializer)?;
            image::RgbaImage::from_raw(width, height, buf.into_owned())
                .ok_or_else(|| {
                    serde::de::Error::custom("Image buffer is smaller than image dimensions")
                })
                .map(Self)
        }
    }
};

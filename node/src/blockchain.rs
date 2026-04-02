use crate::block::Block;
use crate::errors::ProtocolError;

pub struct Blockchain {
    blocks: Vec<Block>,
}

impl Blockchain {
    pub fn new() -> Self {
        Self {
            blocks: vec![Block::genesis()],
        }
    }

    pub fn height(&self) -> u64 {
        self.blocks.last().map_or(0, |b| b.height)
    }

    pub fn append_block(&mut self, block: Block) -> Result<(), ProtocolError> {
        let tip = self
            .blocks
            .last()
            .ok_or_else(|| ProtocolError::StateError(String::from("chain tip missing")))?;

        if block.height != tip.height + 1 {
            return Err(ProtocolError::InvalidBlock(String::from(
                "invalid block height",
            )));
        }

        if block.previous_hash != tip.block_hash {
            return Err(ProtocolError::InvalidBlock(String::from(
                "invalid previous hash",
            )));
        }

        self.blocks.push(block);
        Ok(())
    }
}

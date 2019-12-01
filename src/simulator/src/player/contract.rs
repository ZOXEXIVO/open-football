use crate::player::player::{Player, PlayerPosition};
use crate::core::context::SimulationContext;
use crate::club::tactics::Tactics;
use crate::club::squad::Squad;
pub use chrono::prelude::{NaiveDate, DateTime, Utc, Datelike};
use crate::PlayerEvent;

#[derive(Debug, Clone)]
pub struct PlayerClubContract {
      pub player: Player,
      pub salary: f64,
      pub expired: NaiveDate,
      pub additional_options: AdditionalOptions
}

#[derive(Debug, Clone)]
pub struct AdditionalOptions{
      pub yearly_increase_wage: u16
}

impl PlayerClubContract {
      pub fn new(player: Player, expired: NaiveDate) -> Self {
            PlayerClubContract {
                  player,
                  salary: 100_000.0,
                  expired,
                  additional_options: AdditionalOptions{
                        yearly_increase_wage: 15     
                  }
            }
      }

      pub fn is_expired(&self) -> bool {
            let now = Utc::now();

            let naive_now = NaiveDate::from_ymd(
                  now.year(), now.month(), now.day()
            );
            
            self.expired >= naive_now
      }

      pub fn simulate(&mut self, context: &mut SimulationContext) -> Vec<PlayerEvent> {   
            if context.check_contract_expiration(){
                  if self.is_expired(){
                        
                  }
            }
            
            
            self.player.simulate(context)
      }
}

#[derive(Debug, Clone)]
pub struct PlayerCollection {
      pub players: Vec<PlayerClubContract>
}

impl PlayerCollection {
      pub fn new(players: Vec<PlayerClubContract>) -> Self {
            PlayerCollection {
                  players
            }
      }

      pub fn len(&self) -> usize{
            self.players.len()
      }

      pub fn get_match_squad(&self) -> Squad {
            let players = self
                .players
                .iter()
                .filter(|player_contract| !player_contract.is_expired())
                .map(|p_contract| (PlayerPosition::Goalkeeper, p_contract.player.clone()))
                .collect();

            Squad {
                  tactics: Tactics::new(),
                  players,
            }
      }
      
      pub fn simulate(&mut self, context: &mut SimulationContext) -> Vec<PlayerEvent> {
            self.players.iter_mut()
                .flat_map(|player| player.simulate(context))
                .collect()
      }
}
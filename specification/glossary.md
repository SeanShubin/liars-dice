# Glossary

| Term | Definition |
|------|------------|
| Game | One of the four programs: a presentation delivered to the desktop or to the web. Never a unit of play; that is a match. |
| Presentation | What is seen and how it is touched. Minimal or pretty. |
| Delivery | Where a game runs. Desktop or web. |
| Match | A sequence of rounds played by one unchanging set of players, ending when only one of them remains. |
| Round | A roll, a sequence of turns, a reveal, and a resolution. |
| Roll | Giving every die in play a new face value. |
| Reveal | Showing every player's dice to every player. |
| Resolution | Determining the effect of a round on each player's dice. |
| Turn | A single player making a single claim. |
| Claim | One of: raise, liar, spot on. |
| Raise | A claim that at least a given quantity of a given face value is present among all players, stronger than the previous claim. |
| Spot On | A claim that the previous claim is exact rather than at least. Ends the round. |
| Liar | A claim that the previous claim is false. Ends the round. |
| Quantity | The "at least this many" component of a raise. |
| Face value | The die face component of a raise. One through six. |
| Glyph | The mark on a die face that shows its face value. |
| Turn order | The order in which players take turns. Chosen at random when the match starts, wraps, and is not reordered during a match. |
| Current player | The player whose turn it is. |
| Next player | The player after the current player in turn order. |
| Previous player | The player before the current player in turn order. |
| Exchange | The contest between the player who ends a round and the previous player. |
| Eliminated | A player with no dice, who takes no further part in the match. |
| Ranking | The order of the players of a match by how long they lasted. |
| Beat | To last longer than another player in a match. |
| Design principles | The properties every ruleset and every variant preserves. |
| Default rules | The rules with every variant off. |
| Variant | A single configurable boolean deviation from the default rules, fixed for a whole match. |
| Ones wild | Variant. When on, ones count toward every face value at resolution, and a claim may not name the one face. |
| Relay | The service that orders messages and knows no game rules. |
| Event log | The ordered record of every message in a match. The only state. |
| View | Data derived from the event log, which can change as events are appended. |
| Protocol version | The version of the message format a client speaks. |
| Player | A participant in a match. Either a human player or an AI player. |
| Human player | A player whose claims a person makes. |
| AI player | A player whose claims the program makes. |
| Local player | The human player using this game. |
| Opponent | A player other than the local player. |
| Seat | A player's place in a match, to which dice are dealt. |
| Host | The player who created a match and can start it, bar players from it, and save and load it. |
| Dice limit | Five. The most dice a player can hold. |
| Shield token | Prevents its holder's next die loss, and is spent doing so. |
| Barred | Not allowed to join a match, by the host's designation. |
| Affected | Having lost a die, gained a die, gained a shield token, or spent a shield token as a result of a resolution. |
| Complete | Said of a match in which only one player remains. |
| Administrator | A configured Discord account with access to the data browser. |
| Save | A named record of a match, carrying the time it was made, that the host can load to continue the match. |

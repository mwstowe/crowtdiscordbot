                    // Get the display name - prioritize server nickname over global name
                    let display_name = if let Some(guild_id) = msg.guild_id {
                        // Get member data which includes the nickname
                        if let Ok(member) = guild_id.member(&ctx.http, msg.author.id).await {
                            // Use nickname if available, otherwise fall back to global name or username
                            member.nick.unwrap_or_else(||
                                msg.author.global_name.clone().unwrap_or_else(||
                                    msg.author.name.clone()
                                )
                            )
                        } else {
                            // Fallback if we can't get member data
                            msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone())
                        }
                    } else {
                        // Not in a guild (DM), use global name or username
                        msg.author.global_name.clone().unwrap_or_else(|| msg.author.name.clone())
                    };

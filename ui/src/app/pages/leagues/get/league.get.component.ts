import { Component } from '@angular/core';
import { ActivatedRoute } from '@angular/router';
import { UntilDestroy, untilDestroyed } from '@ngneat/until-destroy';
import { TitleService } from 'src/app/shared/services/title.service';
import { LeagueDto, LeagueService } from '../services/league.service';
import {PageComponent} from "../../page.component";
import {LeftMenuService} from "../../../shared/left-menu/services/left.menu.service";
import {TopHeaderService} from "../../../shared/top-header/services/top.header.service";
import {ProcessService} from "../../../shared/process/services/process.service";

@UntilDestroy()
@Component({
    templateUrl: './league.get.component.html',
    standalone: false,
    styleUrls: ['./league.get.component.scss']
})
export class LeagueGetComponent extends PageComponent {
  public league: LeagueDto | null = null;

  constructor(private leftMenuService: LeftMenuService,
    private topHeaderService: TopHeaderService,
    private service: LeagueService,
    private route: ActivatedRoute,
    private titleService: TitleService,
    private processService: ProcessService) {
    super(processService);
  }

  override onDataRefresh(): void {
    this.route.params.subscribe(params => {
      this.service.get(params["slug"]).pipe(untilDestroyed(this)).subscribe(leagueData => {
        this.league = leagueData;

        this.titleService.setTitle(leagueData.name + ', ' + leagueData.country_name);

        this.topHeaderService.setContent(leagueData.name,
          leagueData.country_name, '/countries/' + leagueData.country_slug);

        this.initLeftMenu(leagueData);
      });
    });
  }

  initLeftMenu(league: LeagueDto) {
    this.leftMenuService.setMenu([
      { items: [{ url: '/', title: 'Home', icon: 'fa-home' }] },
      { items: [{ url: '/countries/' + league.country_slug, title: league.country_name, icon: 'fa-home' }] },
      { items: [{ url: '/leagues' + league.slug, title: league.name, icon: 'fa-home' }] }
    ]);
  }
}
